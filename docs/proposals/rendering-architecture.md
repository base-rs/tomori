# Proposal: A Render World, Visibility, More Lights and Room for Realism

Status: **approved by the maintainer**, November 2026, with one change: the new GPU-free crate is named `tomori-world`, which this text now follows. **What *realistic* means is decided** (section 7): high quality, more than solid geometry and flat surfaces. This proposal's plan for it, physically based, textured materials with ambient and image-based light and baked light on the Quest, real-time extras on the desktop, and Tomori's own interface kept stylised and emissive, revises the stages still to come (section 11). **Stage 1 is built**: bounds, frustum culling against both eyes, back-face culling of closed parts, the sky after the opaque scene, the GPU's frame log and the farmyard's census, now in `design/rendering.md`, which takes precedence; it found closed parts from their triangles rather than from a flag, and bounded skinned meshes by their joints' boxes, and its census is a GPU test of what the renderer culls rather than a count of triangles in view. **Stage 5, fixed foveation, is built and measured**: the runtime's density map, or Tomori's own where it gives none, as the Quest 3's runtime gave none, attached to the eyes' pass by wrapping the device's render pass and framebuffer functions rather than patching wgpu, at high by default, which took the eyes' GPU time from 8.32 ms to 6.72 ms, in `design/rendering.md`. **Stage 2 is under way**: its GPU-free crate, `tomori-world`, holds the render world's roots, items with their instances' bounds, layers, lights and views, and culls them against both eyes' frustum, static instances through a bounding volume hierarchy, without allocating each frame; the renderer reads from it through a thin adapter, its pipelines unchanged, and Tomori's cubes and panels, the farmyard's models, ferns, plaques and lamps, and Sky's platform and clouds, submit to it. Static meshes are chunked: the farmyard's yard, foothills, trail and upper terrace, split as its scenery uploads them into chunks at most 7 m across of their own triangles, each culled and lit on its own. Sorting the draw lists, the rest of Tomori's own things and moving the environment registry out of `Sceneries` are still to come. **Stage 3 is built**: the world chooses the lights of each instance the eyes may see, at most 8 of at most 64 a root, by one function, ranking by brightness at the bounds' centre rather than the item's centre alone and fading at the cut by brightness rather than by reach; the models' and ferns' pipelines read each piece's lamps as a mask with a byte of light each, beside where it is placed, rather than a range of a storage buffer, which cost each pixel more than the lamps it saved, in `design/rendering.md`. With the farmyard's four lamps the scenery is 7% quicker on the trail, 2% at the campsite and 2% slower at the arrival, and with 32 lamps each piece's own cost a half to a sixth less than every lamp, though the yard's ground, trail and foothills, each one piece met by 10 to 27 of them, were then lit by 8, until they were drawn in chunks, each reached by at most 9; of more than 64 lights, those at the cut fade as those at an instance's do. **Stage 4 was tried and closed.** Its LOD (#484) gained too little for its complexity; a generic version may come back later, with coarse levels built in code and no fade. Its multiview (#414) saved the CPU a little but cost the Quest 3's GPU more at the samples Tomori draws, and twice the recommended size with 2× MSAA (#527) was the better answer. **The maintainer has since decided more**, recorded in section 7 and the new section 8:

- textures come from three sources: tiling materials generated offline by code in the repository, hero objects authored offline, and shader detail made on the device;
- the faceted, low-poly look stays, with textures and materials added to it;
- quality tiers over one scene description: a weaker renderer falls back to static or less expensive lights and low-poly shapes, tied into a LOD system, and the renderer supports different pipelines, high quality, low quality and ray traced. This proposal names them Low, High and Ultra, and chooses them by capabilities, a setting or measured frame time.

Reverse-Z is not needed now (#533). Stages 6 to 11 are proposals still; each becomes a requirement as it brings itself into the design documents it changes (listed at the end). It was written against `origin/main` at `1aa32a71` (#371), measured on that commit on the desktop and, read-only, on the maintainer's Quest 3 as they used it.

It builds on the rendering order and the panels kept across frames (`design/rendering.md`, #284, #286), the frame timing the headset and desktop log (`tomori_scene::frame_timing`), environments' client crates and their sceneries (`design/environments.md`, `design/clients.md`), assets drawn with `tomori-assets` (`design/assets.md`), the farmyard and its triangle budgets (`design/farmyard.md`), Sky's baked lightmap and moon shadow map, and the content store accepted in `channels-and-content.md` (#283), which is not built yet.

## The request

Spoken by the maintainer from the headset, in the farmyard, November 2026:

> I think we're at the point where we need to implement a proper rendering system. The graphics are amazing and I love the style, but we're missing an opportunity to do forward rendering with more lights, and the more geometry we have, the more costly it gets to render. I suspect what we need is a better querying mechanism for rendering; frustum culling might be a good first step. And then maybe putting this all into a crate (do we have a render crate already?), something that helps us grow into the next evolution of this project.

The low-poly, flat-shaded look of the farmyard and Sky was to be kept. The maintainer's later decisions (section 7) keep its faceted shapes and add textured, physically based materials to them. Three additions followed:

- **Look to Bevy** for inspiration: its render world and extract phase, its visibility and frustum culling, clustered forward lighting, PBR materials and its renderer's crates. Say what to borrow, and whether to depend on any of it.
- **Don't let the style hold us back:** "It would be nice if we had the ability to do high-quality rendering, whatever the latest is in Unreal Engine-style rendering. I like this style, but it shouldn't hold us back from more realistic rendering. We might need to decide what that is."
- **Bake light on the server:** "We can use the server to pre-compute lightmaps or whatnot. Even if it was somewhat dynamic, we can offload some of that processing from the headset, as long as latency isn't a concern, and maybe there's value in those things being shared across all connected users."

## Recommendation in brief

- **Measure first, and the measurements say the headset's GPU leads.** On the Quest 3, over 20 minutes of the maintainer's session, the GPU took 9.5 ms of each 13.9 ms frame at 72 Hz (77% busy, at 456 to 545 MHz of the Adreno 740's 690). The CPU's frame thread did about 6 to 9 ms of work, 2.2 to 2.9 ms of it encoding the two eyes. Frames were on time: 0.7 a second were shown stale. At 90 Hz the same GPU work would fill about 86% of an 11.1 ms frame at today's clocks.
- **Most of the farmyard is off-screen, but its cost is mostly pixels.** From the arrival, the workbench, the trail and the campsite, 14 to 44% of the scenery's model triangles lie in either eye's view when looking at the scene's subject, and 27 to 29% averaged over all headings. Culling whole objects as they are built today keeps 41 to 70%, because the yard, the foothills and the trail are one mesh each. On the desktop, though, 85 to 90% of the scenery's GPU time scales with pixels, not with geometry. The biggest single GPU saving measured was ordering, not culling: drawing the sky after the opaque scene rather than before it saves 12 to 16% of the scenery's GPU time.
- **A render world, GPU-free, in a new crate `tomori-world`.** Each frame the renderer knows every mesh, instance batch, material, light and view with its bounds. Environments and Tomori's own things (panels, walls, signs, the swarm, workers, rays) submit items to it rather than drawing opaque blocks of commands. `tomori-render` stays the wgpu backend that draws it. The server can link the world crate without wgpu, to bake light from the same description.
- **Visibility:** bounds for every item and instance batch; frustum culling against both eyes, as one combined test; static meshes split into chunks of about 8 m; a BVH over static scenery, rebuilt when the environment changes; dynamic items tested directly; back-face culling where a mesh is closed; occlusion culling not planned. LOD returns as part of the Low tier (section 8), with coarse levels built in code.
- **Lighting:** a light list for each item, nearest-N with a budget, before clustered forward (Forward+). Use 64 lights in a scene and at most 8 for each item, culled by the frustum and by reach. The farmyard's four lamps today fill the shader's fixed `LAMPS = 4`, and every model pixel loops over all four wherever it is: 7 to 13% of the scenery's GPU time on the desktop. Clustering pays for itself only with dozens of lights overlapping in view, as a data centre's racks might have. The style is kept: per-face normals, the stylised `(1 − d/r)²` falloff and its wrap term, the evening palette, now as one shading model among others.
- **Room for realism:** a material says its shading model: `Stylised`, as today, or `Pbr`, glTF's metallic-roughness with normal maps, image-based light from an environment map and tone mapping in the same pass; or it is `Custom`, an environment's own pipeline, as the water, fire and ferns are. What the Quest 3 can do at 72 to 90 Hz in stereo is what shipping Quest titles do: PBR with a handful of real-time lights, baked lighting and probes, and no post-processing chain. Real-time global illumination and Nanite- or Lumen-like systems are out of reach there. The desktop and spectator can do more. **The maintainer has chosen high quality** (section 7). Environments' scenery keeps its faceted, low-poly shapes and gains physically based, textured materials (albedo, normal, occlusion, roughness and metalness, emissive), lit by ambient and image-based light, with light baked for the Quest. Tomori's own swarm, panels and signs keep their stylised, emissive look.
- **Textures from three sources** (section 7):
  - tiling surface materials generated offline by code in the repository, compressed to ASTC with mips and served from the content store;
  - hero objects authored offline, with provenance and licence tracked;
  - shader detail and animation made on the device, with no stored texture.
- **Render tiers** (section 8): one scene description, the render world, feeds interchangeable render paths chosen for each device and setting:
  - **Low:** static or few lights, simple materials, coarser LOD;
  - **High:** the Quest 3's target, as above;
  - **Ultra:** the desktop's, with ray-traced or hybrid shadows, reflections and GI where the GPU allows.

  Materials are authored once, and each path reads them at its own fidelity.
- **Server-side baking fits Tomori well.** The server already links every environment's models, built in code without a GPU, and Sky already bakes its lightmap at build time. Bake per-face irradiance for the stylised path, needing no lightmap UVs, about 0.2 MB for the farmyard. For PBR, bake lightmap atlases, and for everything that moves, a probe grid. Each light that can switch keeps a layer of its own, so the campfire's flicker scales its layer live. Results are addressed by the hash of what they were baked from, served from the content store and shared by everyone in the space.
- **Don't build on Bevy's render crates.** `bevy_render` 0.20 is on wgpu 30 and glam 0.33, as Tomori is, so versions are not what stops it. Every one of its render crates brings Bevy's ECS, app, asset system and schedules, and a desktop-shaped pipeline of extra passes. Borrow its designs instead: extract, then prepare, then queue; `Aabb`, `VisibleEntities` and `VisibilityRange`; the split of `bevy_camera` and `bevy_light` from the GPU crates; its clustering layout for later; and its PBR formulas.
- **Eleven stages, each shippable** (section 11, revised after the decisions):
  1. bounds, frustum culling, back-face culling, the sky drawn last and a profiler with budgets in CI (built);
  2. the render world, chunked static scenery and its BVH (under way);
  3. light lists and a light budget (built);
  4. LOD and multiview (both tried and closed);
  5. foveation (built);
  6. materials in the render world, and the tier abstraction;
  7. the procedural texture generator, and delivery from the content store;
  8. PBR shading in High;
  9. Low's fallbacks and generic LOD;
  10. baked light;
  11. Ultra.

## Measurements

### How they were taken

- **The headset:** read-only `adb logcat` of the running app's own `tomori::xr` frame log and the runtime's `VrApi` once-a-second line, and `/sys/class/kgsl` for the GPU's frequencies. Nothing was installed, started or stopped. The session ran from 14:28 to 14:49 NZDT on 12 November 2026, in the space "Tomori development". The agent's speech in the log says the maintainer had rejoined the farmyard; which part of it they stood in is not logged. There were 1,238 one-second samples and 15 one-minute frame summaries.
- **A census of the farmyard's geometry,** on the CPU from the exact meshes its common crate builds. A throwaway example built every model, instance, fern and range, with its world-space triangles. For four viewpoints it then counted what falls in either of two Quest-3-like eye frusta (outer 52°, inner 41°, up 40°, down 50°, eyes 64 mm apart, gaze 10° below level). Each viewpoint was measured facing its subject and averaged over eight headings, together with back-facing triangles and depth complexity from a small software rasteriser at 210 × 220.
  - **Arrival:** at `layout::ARRIVAL`, facing `ARRIVAL_FACING`.
  - **Workbench:** 1.7 m out from the bench's corner, looking into it.
  - **Trail:** at station 3, looking along the trail.
  - **Campsite:** 2.2 m from the fire, eyes at 1.2 m as if seated.

  Hens were counted at the flock's most, five. The beach's physics stones and the sticks, which the client side places, were not counted.
- **The scenery's GPU time on the desktop,** an RTX 4090 under Vulkan. The farmyard's `Scenery` was drawn as the headset draws it: two passes, one per eye, at the Quest 3's 1,680 × 1,760 per eye with 4× MSAA resolved in the pass. Pass timestamps gave the median of 120 frames for each viewpoint, with each part of the scenery left out in turn. This measures the scenery alone, without Tomori's panels, signs or swarm. A desktop GPU draws immediately where the Adreno bins into tiles, so its ratios are a guide to the Quest's, not a measurement of them.

Both examples were throwaway and are not in this change; stage 1 proposes committing the census as a test.

### The headset (Quest 3, 72 Hz, 1,680 × 1,760 per eye, 4× MSAA, fixed foveation off)

| What | Measured |
| --- | --- |
| Frames | 71 to 73 of 72 each second; 0.7 a second shown stale; 6 to 22 refreshes missed a minute |
| GPU time a frame (`VrApi` `App`) | mean 9.53 ms, median 9.58, 5th percentile 8.69, 95th 10.35 |
| GPU busy, and its clock | 77% mean, mostly at levels 2 to 4: 456, 492 or 545 MHz, of 690 MHz at most |
| CPU frame thread, its work | 5.7 to 10.1 ms a frame on average by the minute, including waits |
| … encoding both eyes (`render`) | 2.2 to 2.9 ms mean, 2.5 to 3.9 ms at the 95th percentile |
| … applying the frame (`draw`), the frame step, the projector | 0.7 to 0.9, 0.6 and 0.2 ms |
| … waiting for the GPU's last frame | 1.6 to 4.5 ms mean, 9.5 to 10.2 ms at the 95th percentile |
| Temperature | 45 to 46 °C |

The headset makes its frames at 72 Hz, with the GPU about three-quarters busy at moderate clocks. Saving GPU time today buys lower clocks, so less heat and battery, and room: for more lights and geometry, for PBR, or for 90 Hz, where 9.5 ms is 86% of the frame. The CPU's 2.2 to 2.9 ms of encoding is the largest part of its own work. It comes from each eye being a pass of its own, every draw encoded twice, and from each panel's matrix being written once for each eye.

**Correcting what was said in the headset:** the agent said earlier that Tomori draws with multiview. It does not. The swapchain is an array of two layers (`xr.rs`, `array_size: 2`), but `Renderer::render` begins one render pass for each eye and encodes every draw in both. Every pipeline has `multiview_mask: None`.

### The farmyard's geometry

| Part | Triangles | Pieces (instance × primitive) |
| --- | ---: | ---: |
| The yard's props | 4,040 | 2 |
| The coop | 722 | 2 |
| Five hens | 6,300 (1,260 each) | 10 |
| The campsite, the beach's shingle | 355, 432 | 2, 1 |
| The trail | 1,144 | 1 |
| The foothills | 2,880 | 1 |
| The bush (five kinds of plant) | 10,862 | 318 |
| The river's stones, the waterfall rock | 765, 40 | 51, 1 |
| The water | 197 | 1 draw |
| **Models and water** | **27,737** | **18 model draw calls** |
| Ferns, 192 placed, near and far detail | 17,228 to 20,153, by where the eyes are | 13 meshes, one draw each with instances |
| The ranges, at the sky's depth | 1,184 | 1 |

That is 46,000 to 49,000 triangles for each eye in about 40 of the scenery's own draw calls, encoded twice a frame. The rest of each pass is Tomori's own: the swarm's and markers' cubes in one instanced draw, the controllers, each sign, and each panel's face and its occluder. Each environment's tests keep a triangle budget: backdrop 22,000, ferns 24,000, beach 1,400, campsite 800. None counts draw calls, apart from the ferns' at most one draw for each mesh, and none counts what is in view.

**Five hens cost more than the yard:** 1,260 triangles each, for a bird 40 cm tall. That is the clearest case for LOD.

### How much is in view

| Viewpoint | Facing: pieces in view (their triangles) | Facing: triangles in view | Facing: ferns in view | Eight headings: pieces (triangles) | Eight headings: triangles | Eight headings: ferns |
| --- | --- | ---: | ---: | --- | ---: | ---: |
| Arrival | 135 of 393 (70%) | 44% | 44% | 124 (55%) | 27% | 33% |
| Workbench | 97 (41%) | 19% | 26% | 126 (55%) | 27% | 34% |
| Trail, station 3 | 114 (44%) | 20% | 39% | 127 (53%) | 29% | 35% |
| Campsite | 91 (42%) | 14% | 37% | 128 (53%) | 29% | 35% |

- **Culling whole pieces** as they are built today keeps 41 to 70% of the model triangles when facing, and 53 to 55% over all headings. The yard, the foothills and the trail are each one mesh, so they are always in view. Chunking them brings the share towards the triangles' own 14 to 44%, and 27 to 29% over all headings.
- **Ferns culled one by one** keep 26 to 44%. Their two details already cut their triangles beyond 10 m (`ferns::NEAR`).
- **Draw calls:** of the 18 model draws, 9 to 16 have anything in view when facing, and 12 or 13 over all headings.
- **Back faces:** 44 to 51% of the triangles in view face away from the eye. Back-face culling would skip them where a mesh is closed.
- **Depth complexity:** in the left eye, 2.4 to 4.1 triangles cover each pixel. With a perfect front-to-back order and the depth test, 1.2 to 1.7 would be shaded. Then the sky adds one more layer, since it is drawn first, over every pixel, without depth.

### The scenery's GPU time on the desktop

Milliseconds for both eyes, at 1,680 × 1,760 with 4× MSAA, the median of 120 frames. Each cell says how much faster the frame was with that part left out, or changed.

| Viewpoint | Whole scenery | Sky | Sky drawn after the opaque scene | Four lamps | Near models | Far models | Ferns |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Arrival | 0.217 to 0.237 | −0.034 | −0.028 | −0.019 | −0.108 | −0.023 | −0.009 |
| Workbench | 0.200 | −0.030 | −0.024 | −0.015 | −0.078 | −0.023 | −0.002 |
| Trail | 0.174 | −0.033 | −0.028 | −0.022 | −0.045 | −0.031 | −0.007 |
| Campsite | 0.197 | −0.032 | −0.030 | −0.024 | −0.057 | −0.037 | −0.013 |

The near models are the yard, the coop, the hens, the campsite, the beach and the trail. The far models are the foothills, the bush, the river's stones and the rock.

- Clearing and resolving the samples alone take 0.035 ms; the water, the ranges and the fire take next to nothing, except the fire at the arrival, 0.02 ms.
- **Pixels decide:**

  | Size | Time |
  | --- | ---: |
  | 1,680 × 1,760 without MSAA | 0.130 to 0.154 ms |
  | 420 × 440 | 0.030 ms |
  | 105 × 110 | 0.020 to 0.022 ms |

  So geometry, draws and fixed costs are about 10 to 15% of the scenery's time at full size, and the rest scales with pixels and samples.
- **Ordering is the cheapest win.** The sky's full-screen triangle is drawn before everything, without depth, so its shader runs on every pixel. The models then cover 82 to 97% of them. Drawing it after the opaque scene, at the far plane with `LessEqual` and before the ranges, saves 12 to 16% of the scenery's time.
- **The lamps cost wherever they cannot reach.** The four lamps cost 8 to 13% of the scenery's time, the most on the trail, which none of them lights, since every model pixel loops over all four.

What the desktop cannot say is how the headset's 9.5 ms divides between the scenery, the panels, the signs and the swarm. Stage 1's profiler is to answer it.

## What the system has today

| Part | Today | Where |
| --- | --- | --- |
| The renderer | `graphics::Renderer`, shared by the headset, the desktop window and the spectator. For each eye it begins its own pass: each scenery's `draw`, Tomori's cubes in one instanced draw, the controllers, the scenery's `draw_translucent`, the signs and panels merged back to front, arrival's labels and the blink | `crates/render/src/graphics.rs` |
| Applying a frame | `scene_graphics::draw` turns the frame step's `Frame` into the renderer's state, keeping each panel's mesh for as long as its face is the same | `crates/render/src/scene_graphics.rs`, `rendering.md` |
| Sceneries | A `Scenery` trait object per environment: `animate` once a frame, `prepare_eye` and `encode` before each eye's pass, `draw` and `draw_translucent` in it. The renderer cannot see inside it: no bounds, no lights, no sorting | `crates/render/src/scenery.rs` |
| Models | `tomori-assets`' `gpu::Renderer`. Each model's placements, with joints, go into a storage buffer each time they are placed, and there is one `draw_indexed` for each primitive with its instances. Two-sided, lit by a sky and ground ambient, the moon, haze and at most four lamps (`LAMPS`). A mesh without normals is lit flat, by each triangle's own normal from derivatives | `crates/assets/src/gpu.rs`, `model.wgsl` |
| The farmyard | The sky first, then the models nearest first, its own ferns pipeline with two details, the backdrop, the water, the ranges at the sky's depth, the fire added last, and the plaques among the translucent surfaces. Four lamps: the coop's lantern, the gate's, the workbench's and the bonfire, flickering | `crates/environment-farmyard-client/src/scenery.rs` |
| Sky | Build-time baked AO and lantern irradiance, ray-cast on the CPU in `build.rs`; one 1,024² moon shadow map rendered once for its immutable scenery; half-resolution volumetric clouds ray-marched in passes of their own for each eye and composited | `crates/environment-sky-client` |
| Culling | None. Every pipeline has `cull_mode: None`. The only bounds are the navmesh's, the stage layout's on the server, `Primitive::bounds` and the stones' colliders | |
| Instancing | Models' placements; ferns; Tomori's cubes | as above |
| Translucency | The rules of `rendering.md`: opaque first, then translucent surfaces back to front with depth tested, occluders writing depth where at least 60% opaque, and the `TIE` | `occlusion.rs`, `ui_panel.rs`, `workspace_graphics.rs` |
| The Quest | One projection layer; 4× MSAA with transient attachments, resolved in the pass and discarded; no fixed foveation; no multiview; no depth submitted; the runtime chooses the CPU and GPU levels | `crates/client-quest/src/xr.rs` |
| Frame timing | CPU parts of each frame logged once a minute, with counts of faces, meshes and glyphs; no GPU timings of Tomori's own | `tomori_scene::frame_timing` |

### What `crates/render` holds, and what doesn't belong there

`tomori-render` is the wgpu backend for everything Tomori draws itself, and it holds the shaders and pipelines it should hold:

- the swarm, Tomori's cubes, arrival, workers' signs, panels and their images;
- the glyph atlas's GPU side and the controllers;
- the `Scenery` trait.

It also holds things that are not rendering:

- **`Sceneries` is an application's registry of environments.** Beside the sceneries, it holds each environment's locomotion profile, its client side's factory, its ambience, its worker placement, its avatars, and the licences shown with a build's assets. They live there because they are registered by the same identifier. They belong in an environment registry in `tomori-scene`, beside `ClientSide` and `Avatar`, with `tomori-render` keeping only the sceneries.
- **`controller.rs`** loads the controllers' glTF and transcodes KTX2 with Basis Universal. That is asset loading, which `tomori-assets` does for every other model.
- **`panel_images.rs`** decodes images as well as drawing them.

There is no render world: no list of what is drawn, with its bounds, that a query could cull, sort or light. That is the gap the maintainer named.

## 1. The render world

### What the renderer knows each frame

A `RenderWorld`, retained across frames and changed by what changes, holds:

- **Meshes:** vertex and index data with their bounds, uploaded once and named by a handle. Static meshes are split into chunks when they are added (section 2).
- **Materials:**
  - their shading model, `Stylised`, `Pbr` or `Custom(pipeline)`;
  - their parameters, authored once, which each tier's render path reads at its own fidelity (section 8);
  - whether they are double-sided, cut out or blended;
  - for a custom one, its pipeline, made by the environment, and the vertex layout it reads.
- **Items:** a mesh drawn with a material, one or many instances, each with its transform.
  - Each has bounds in the world: a box, and a sphere for quick tests.
  - It is static, rebuilt only when the environment changes, or dynamic, moved each frame.
  - It sits in a layer: opaque, cut-out, sky, translucent or overlay.
  - It may set flags: no frustum culling, which LOD it belongs to, whether it casts shadows.
- **Lights:** each with its position, its reach, colour and intensity, its falloff, `Stylised` or physical, its layer for baked light, and whether it is static (section 6).
- **Environment light:** the sun or moon, the sky and ground ambient, haze, and, for PBR, an environment map and its irradiance.
- **Views:** this frame's eyes, two on the headset and one on the desktop and spectator, each with its view, projection and position, and their combined frustum.

Each frame, the renderer runs Bevy's phases in its own way:

1. **Extract:** take what changed from the frame and the sceneries.
2. **Prepare:** move dynamic items, then cull against the views, choose each item's LOD and assign each item its lights.
3. **Queue:** make sorted draw lists: opaque by pipeline, material and distance; translucent back to front, by the rules `rendering.md` sets.
4. **Draw:** encode the lists once for each eye, by the device's render path (section 8).

The renderer then counts what it drew.

### How things submit to it

- **Environments.** A scenery adds its meshes, materials and lights to the world once, as it is made, and moves its dynamic items in `animate`: the hens, the sticks, the stones. Custom pipelines stay the environment's, as the water's, the fire's and the ferns' do now, but are drawn as items, with bounds, so they are culled and sorted like the rest. A scenery that must do its own passes, as Sky's clouds do, keeps `encode` and gets a `Custom` item in the sky layer. `Scenery::draw` remains for one release as the escape hatch, drawn unculled where it is today, so that downstream environments, such as the observe environments in the headset's log, can move over at their own pace.
- **Tomori's own things.** The swarm and its workers, the home ring, pads and landing ring, now cubes rewritten into one instance buffer each frame, become items with bounds. The controllers and rays become dynamic items. The panels, walls, signs, cards and labels are translucent items whose order is still the frame step's: the render world culls a panel outside both eyes and keeps the order of the rest. Arrival's labels and the hand menu's stay last, as `rendering.md` requires. Nothing about which surface is in front changes.
- **The avatar's scenery** draws in the space's coordinates and submits the same way.

### Crates

| Crate | Holds | Depends on |
| --- | --- | --- |
| **`tomori-world`**, new, in `crates/world` | the render world's types; bounds, frustums and views, the combined stereo frustum, the BVH, LOD selection, light assignment and budgets, draw-list sorting, the census of what is in view; no GPU code | `glam`, `tomori-assets` without its `gpu` feature |
| `tomori-render` | the wgpu backend: pipelines and shaders for the stylised and PBR shading models, a render path for each tier (section 8), the draw lists encoded, the profiler's GPU queries, Tomori's own surfaces, the `Scenery` trait, which now submits to the world | `tomori-world`, wgpu |
| `tomori-assets` | models, glTF, materials, gaining PBR's textures and `doubleSided`; its `gpu` drawing moves into `tomori-render`'s backend as one pipeline among others | |
| `tomori-scene` | gains the environment registry moved out of `Sceneries` | |
| `tomori-lighting`, later, in `crates/lighting` | the baker: per-face irradiance, lightmaps, probes, on the CPU and, optionally, with wgpu compute | `tomori-world` |

A crate apart makes sense because the server needs the world without wgpu, to bake light from it, and so do tests that check culling and budgets on CI without a GPU, as `tomori-environment-farmyard`'s models are built without one now. This is the split Bevy made in 0.17 to 0.20, moving `bevy_camera`, `bevy_light`, `bevy_mesh` and `bevy_material` out of `bevy_render` and `bevy_pbr`. `tomori-render` could instead have been renamed and the backend made a new crate, but its name is published, and downstream sceneries already depend on it. The new crate follows `design/crates.md`'s naming and is published, since an environment's common crate will describe its static scene with it for the server to bake.

## 2. Visibility

- **Bounds everywhere.** Each mesh has its local box, from `Primitive::bounds`. Each instance batch has the union of its instances' boxes, kept as they move, and each instance its own. Skinned meshes, the hens, take a box grown to cover their animation, measured once, as `Chicken::new` already measures the walk's tallest pose. Panels and walls have their quads' boxes, which the projection knows.
- **Frustum culling against both eyes.** Each item is tested against one frustum enclosing both eyes. Its apex lies behind the eyes, where the outer planes of the left and right frusta meet; its near, top and bottom planes are both eyes'. It is one test where two would nearly double the work, and conservative. The desktop and spectator have one view and test against it. Instance batches are culled in two levels: the batch's bounds, then each instance's, writing only the visible placements into the storage buffer. At the farmyard's 400 instances that is about 30 KB a frame.
- **Chunks for big static meshes.** The yard, the foothills and the trail are each one mesh, and so always in view. The world splits static meshes into chunks of about 8 m across, by triangle centroid, each its own item with its own bounds. This keeps each chunk's draw small enough to cull and big enough to be cheap to draw. It narrows the gap the census shows between pieces (53 to 55% in view over all headings) and triangles (27 to 29%).
- **A spatial structure for static scenery: a BVH.** It is built once with binned SAH, over the static items' boxes, as a flat array, when an environment is made or its static scenery changes. With 400 to 2,000 items it is built in well under a millisecond and walked in a few microseconds.
  - **A loose grid** on the ground plane would do for the farmyard, which is flat. But it suits neither tall or stacked scenes, such as a data centre's racks, nor items of very different sizes, such as the foothills beside a fern.
  - **An octree** adapts less well than a BVH to where the items are, and has more nodes to walk. Its advantage, inserting and removing cheaply, matters little when the dynamic items are few.

  So dynamic items, the hens, stones, sticks, the agent, workers, panels and controllers, a few hundred at most, are tested one by one. A second BVH, refitted each frame, waits until there are thousands.
- **Back-face culling where the geometry is closed.** About half the triangles in view face away. The material says whether it is double-sided: glTF's `doubleSided`, and a flag on models built in code. Closed meshes such as the coop, the hay, the stones and the fence posts are culled with `cull_mode: Back`. Leaves, flax, ferns, whose undersides are coloured, the water and panels stay two-sided. Each environment's tests check that a model marked closed has every edge shared by two faces, wound consistently. This saves rasterising and binning, not much shading, as back faces mostly fail the depth test already; it is a modest win.
- **LOD.** An item may name nearer and farther meshes with distances, Bevy's `VisibilityRange`, crossing over with a dithered fade over a metre so that nothing pops. With 4× MSAA the dither can use alpha to coverage. The candidates, in the order the census ranks them:
  - the hens, 1,260 triangles each, under 300 beyond 6 m;
  - the bush's plants beyond 20 m;
  - the ferns, which already have two details, moved onto the general mechanism;
  - the backdrop, whose far plants could become cards.

  The style makes LOD cheap, since a low-poly model's farther detail is the same model with fewer facets.

  **Tried and closed** (#484): simplifying models at startup, choosing by pixels of error with a dithered fade, gained too little for its complexity, about 3,300 lines. Its review asked that a later version build coarse levels in code with the models, switch at a pixel with hysteresis and no fade, and measure ferns from their heart. Section 8 brings it back that way, as part of the Low tier.
- **Occlusion culling: not planned.** The farmyard is open, and the coop and fence hide little. Hardware occlusion queries cost a round trip and a pass on a tiler, and GPU hierarchical-Z culling needs compute passes the Quest can ill afford. It is worth revisiting only for an indoor environment, with portals or cells, which are cheap and CPU-side.
- **The reveal sphere.** While a scenery grows through the reveal sphere, items wholly outside it are culled too.

## 3. Lighting

### More lights, within budget

- **Light lists for each item (nearest-N).** In prepare, each visible item gets the lights whose sphere of reach meets its bounds. If there are more than the item's budget, the brightest at the item's centre are kept, those near the cut fading out over 20% of their reach so that nothing flickers as the head moves. The lists go to the shader as a small range into a storage buffer of light indices. A light whose sphere is outside both eyes is not uploaded.
- **Budgets:** 64 lights in a scene, at most 8 for each item, and in the shader a loop over the item's own list, not the scene's. Most of the farmyard's pixels, on the backdrop, would loop over none, recovering most of the 8 to 13% the lamps cost now. The coop could then have a lantern inside, the trail lanterns at its stations, and the campsite more than one light, without each pixel paying for all of them.
- **Clustered forward (Forward+), later if needed.** Divide each view into a froxel grid, say 16 × 9 × 24, and assign lights to clusters on the CPU, as Bevy does. Each fragment then loops over its cluster's lights. It is exact per pixel where nearest-N is exact per item, and it pays off when many lights overlap a big item, such as a long wall of rack lights or a wide floor lit by many lamps. With a few dozen lights it costs more than it saves. On the Quest it costs a storage buffer per view, rebuilt each frame, and the cluster lookup per fragment.

  The world's light assignment is one function, so changing to clusters later changes neither the environments nor the lights they declare. The proposal is nearest-N in stage 3, and clusters when an environment's census shows items with more than 8 lights in reach.

### Keeping the style

Stylised stays the shading model of Tomori's own things, and the farmyard's and Sky's until stage 8 gives their scenery physically based materials (section 7):

- normals from each face, as models built in code give them and as the shader derives them where a mesh has none;
- light falloff `(1 − d/r)²` with the wrap term `0.35 + 0.65 N·L`, so that lamplight reaches round the sides of facets;
- the sky and ground ambient and the haze;
- the evening palette, unchanged.

The light list changes which lamps a pixel loops over, not how each lamp looks. Within reach and within budget, each lamp lights exactly as now, which the farmyard's render tests check pixel for pixel.

### Shadows

- **Today:** Sky renders its moon shadow map once, for scenery that never moves. The farmyard has no shadows.
- **On the headset:** static shadows from the baked light (section 6), for the moon and the lamps alike, and for dynamic objects a soft blob beneath them, which suits the style and costs one quad. A real-time shadow map for the moon, covering only a few metres about the participant, drawn only by dynamic items, is a later option of about one depth pass a frame.
- **No real-time point-light shadows on the Quest:** a cube map is six passes for each light.

## 4. Batching

- **Sorting.** Opaque lists are sorted by pipeline, then by material, then front to back. The census shows 2.4 to 4.1 layers over each pixel, and 1.2 to 1.7 shaded in a perfect order; front to back by item gets most of the way. Opaque items come first, then cut-outs, then the sky (its shader runs only where nothing nearer was drawn), then the ranges and translucent items, as `rendering.md` sets them.
- **Merging static scenery** by material within each chunk, at build time, so a chunk is one draw for each material. Instancing stays for repeated models, the bush, ferns and stones, culled at the instance (section 2).
- **Fewer state changes:** one bind group of the view for each pass, materials in one buffer indexed by the draw, and placements for every model in one storage buffer with offsets, rather than a buffer for each batch. Bevy's `offset-allocator`, a standalone crate, can manage the suballocation.
- **Panels and text.** Each panel's face is a draw plus its occluder's, and its matrix is written once for each eye. Culling panels outside both eyes saves both. Writing every panel's matrix into one buffer, indexed by the draw, removes the writes for each eye, as multiview does. The order and the occluders are kept, and text stays a mesh of glyph quads from the atlas, as `text.md` requires.

## 5. The Quest

- **The frame:** 13.9 ms at 72 Hz, 11.1 ms at 90 Hz, 8.3 ms at 120 Hz, for the CPU and the GPU each, since they overlap (`rendering.md`). Today's GPU 9.5 ms fits 72 Hz with headroom and 90 Hz only at higher clocks. So the plan's GPU savings go first to headroom, then to features, and to 90 Hz if the maintainer wants it (open questions).
- **No extra passes.** On a tiled GPU, each pass writes its tiles to memory and the next reads them back. Tone mapping, fog and colour grading go in each material's fragment shader, writing straight to the sRGB swapchain. There is no HDR target, no separate post-processing, no depth prepass and no screen-space effects. This is the main reason Bevy's default 3D pipeline does not suit the headset as it is.
- **Resolution and MSAA:** resolved in the pass into the swapchain, with transient multisampled colour and depth discarded at its end. A Quest 3 now draws each eye at twice the recommended size with 2× MSAA (#527, `rendering.md`), and other devices at the recommended size with 4×.
- **Multiview: tried and closed** (#414). Drawing both eyes in one pass saved the CPU a little, but on the Quest 3 it cost the GPU more at the samples Tomori draws, since its tiles shrink under MSAA: in Observe, 7.0 ms for multiview at 2× against 5.8 ms for each eye at 2×. Resolution with 2× MSAA was the better answer. The branch is kept, to reopen if a scene turns out to be CPU-bound.
- **Depth: reverse-Z is not needed now** (#533). A 32-bit float depth buffer with normal Z is precise enough at every distance Tomori draws, about 1 mm at 30 m and 5 cm at 200 m (#397). It is worth revisiting, with an infinite far plane, for the Ultra tier's shadow maps (section 8) or a far plane much beyond 2 km.
- **Fixed foveated rendering.** The runtime reports `Fov=0`, so it is off. `XR_FB_foveation` lowers the shading rate towards the edges of the view, which typically saves 10 to 25% of fragment work, and fragment work is most of the GPU's. On Vulkan it needs the runtime's fragment density map attached to the render pass (`VK_EXT_fragment_density_map`), which wgpu does not expose. It needs either a small patch to wgpu's Vulkan backend, carried as a `[patch]` in the workspace until upstream has something like it, or the pass begun through wgpu-hal. This is the largest GPU saving on the headset that does not touch content, and the riskiest to build.
- **Thermal headroom.** The runtime set the GPU at levels 2 to 4, not 5, for 9.5 ms at 77% busy. Tomori never sets `XR_EXT_performance_settings` (`rendering.md`'s open question). Lower GPU time lets the runtime stay at lower levels for longer sessions. The profiler's log should keep the runtime's levels beside Tomori's own times.
- **Later:** depth submitted for reprojection (`XR_KHR_composition_layer_depth`), and Application SpaceWarp, which needs motion vectors. Neither is needed while frames are on time.

## 6. Server-side light baking

### Why it fits

The server already links every environment's common crate, and the farmyard's models are built in code there without a GPU. Sky already bakes AO and lantern light at build time, on the CPU, ray-casting against the boxes its mesh is made of. A bake on the server keeps the headset's cost at nothing beyond sampling. It can follow the environment's changes, as a build-time bake cannot: the campfire lit, a lantern moved, the time of day, a cairn built. Done once for each space, it is shared by every participant, the spectator and the desktop window alike, by hash.

### What is baked, and what stays real-time

| | Baked on the server | Real-time on the headset |
| --- | --- | --- |
| Static scenery, stylised | irradiance for each face, a colour a triangle, by light layer | the light layers' live intensities; nearby dynamic lights not baked |
| Static scenery, PBR | lightmap atlases (glTF `TEXCOORD_1`) of irradiance, by layer; reflection probes for shiny materials | direct light from the item's light list where specular matters; sampling the probes |
| Dynamic objects: stones, hens, sticks, the agent, workers, avatars | a grid of probes (irradiance as L1 spherical harmonics) over the walkable volume | sampling the probes at the object's centre; its light list; a blob shadow |
| The campfire, lanterns that can switch | each switchable light's bounce and occlusion as a layer of its own | the layer scaled by the light's live intensity, the flicker each frame, its direct light real-time |

- **Per-face irradiance suits the style exactly.** Flat-shaded triangles already have vertices of their own, so a baked colour for each face needs no lightmap UVs, no atlas, no seams and no filtering. It is an extra vertex attribute, 4 bytes a triangle (RGB9E5 or RGBM8): about 190 KB for the farmyard's 47,000 triangles and each light layer. The look stays flat, with light bounced from the fire and occlusion beneath the coop.
- **Probes:** L1 spherical harmonics, 12 half floats, 24 bytes, at 1 m spacing over the farmyard's 40 × 40 × 3 m of walkable space: about 4,800 probes, 115 KB. They are sampled at an object's centre and blended trilinearly.
- **Lightmaps for PBR:** 1 to 4 MB an environment, transcoded on the client to ASTC on the Quest and BC6H on the desktop.

### Invalidation, latency and determinism

- **What a bake is named by.** A bake's input is the static scene, its meshes, transforms, materials and static lights, as `tomori-world` describes it, with the bake's settings and the baker's version. Its hash names the input, and the output is stored by its own hash in the content store, `GET /content/{hash}`. A client that has it already, from an earlier session, never fetches it again.
- **Determinism:** fixed seeds and a fixed order of samples for each face, so that the same input gives the same bytes on any server, and caches agree.
- **Incremental.** A change dirties what is within its reach: a cairn, the faces and probes within about 3 m of it; a lantern moved, those within its reach. Only those are baked again, and the client receives a patch keyed by the base bake's hash.
- **Latency.** Light may lag geometry by seconds. The previous bake stays until the new one arrives, then cross-fades over half a second. Switching a light is never a bake, since its layer is scaled live. Time of day is a few bakes at key times, blended by the client.

### Cost on the server

- **The CPU path:** a path tracer over a BVH of the static triangles. The farmyard is about 47,000 faces at 256 rays each, about 12 million rays. On the server's 16 cores at a conservative 10 to 50 million rays a second, that is 1 second or less, cut further by baking incrementally. The BVH is `tomori-world`'s own, and parallelism is `rayon`'s. Embree's bindings would be faster but bring a C++ library.
- **The GPU path, optional:** wgpu compute with ray queries, which wgpu 30 has as an experimental feature on Vulkan, used when the server has a GPU and falling back to the CPU when it has none. Not needed for scenes the size of the farmyard.
- **Who bakes:** the server's core, not each environment's server side. A side declares its static scene once, and changes to it as they happen, through the state channel's existing path. The core bakes on a thread of its own, at low priority, with a bound on the time it may take for each bake.

### Streaming and the content store

Baking depends on the content store's stage 1 (`channels-and-content.md`), which is accepted and not yet built. It adds kinds to the store's allow-list:

- `application/vnd.tomori.face-light`, per-face irradiance by layer;
- `application/vnd.tomori.probes`;
- KTX2 lightmaps.

Each has a size bound, as the other kinds do. A space's state names its environment's current bake by reference. Clients fetch it when they enter, keep drawing without it, unlit by baked light, as today, until it arrives, and swap it in with a fade. The store's writes stay the server's own, and garbage collection by reference keeps a bake while a space names it.

### Precedent and alternative

Build-time baking, as Sky does, is simplest and stays right for scenery that never changes. The server bake is for what changes, and for environments the client doesn't build in. A client baking for itself is ruled out: the Quest's CPU and GPU are what this saves.

## 7. What realistic should mean for Tomori

### The decision

The maintainer decided, in November 2026:

> Realistic rendering should be high quality — we should aim for more than just solid geometry and flat surfaces.

Then, reviewing this plan:

- **The look:** keep the current faceted, low-poly style, adding textures and materials to it, rather than moving to smooth geometry.
- **Where textures come from:** the hybrid below ("seems okay").
- **Weaker devices:** "A weaker renderer should fall back to static / less expensive lights; low-poly shapes also okay, perhaps tied into a LOD system… ideally in a way that supports different rendering pipelines, e.g. high quality vs low quality vs ray traced. Open to ideas." Section 8 proposes the tiers.

The rest of this section is this proposal's plan, not the maintainer's words. It reads the decision as option 3 below: every device gets the same scenes and materials, the Quest 3 draws them as shipping Quest titles do in the High tier, and the desktop and spectator add what only they can afford in Ultra. This replaces the earlier recommendation, which was option 1 for the farmyard and Sky.

### The plan: what high quality means

- **Physically based materials, textured.** Every surface of an environment's scenery takes glTF's metallic-roughness material, each part a texture or a constant:
  - base colour (albedo);
  - a tangent-space normal map;
  - occlusion, roughness and metalness, packed into one texture (ORM);
  - emissive.

  **The shapes stay faceted.** Each face keeps its own normal, and its texture and normal map add grain, wear and detail within it, so a facet catches the light as one plane with texture on it, not as a smoothed curve. Geometry is added only where a silhouette shows it, as low-poly as before.
- **Ambient and image-based light.** Each environment's sky is captured into a small environment map: a prefiltered 128² cube, its mips running from smooth reflection to rough, and its irradiance as spherical harmonics. Every surface is then lit and reflected by its own sky and ground, by its roughness and Fresnel. The occlusion map also dims reflections, keeping crevices dark.
- **Baked light on the Quest** (section 6): indirect light and occlusion for static scenery in lightmaps, one for each light layer; probes for what moves; and reflection probes where something is shiny or wet. Direct light comes in real time from the moon and each piece's own lamps, at most 8, as stage 3 chooses them.
- **Tone mapping and exposure** in each material's own shader, with an exposure set by each environment. The headset gets no extra passes (section 5).
- **Real-time extras on the desktop and spectator,** the Ultra tier (section 8): an HDR target with bloom, ray-traced or hybrid shadows, reflections and GI where the GPU allows, and more lights by clustered forward. A participant at the desktop and one in the headset see the same things, the desktop's more finely lit.
- **Tomori's own things keep their look.** The swarm and its workers, panels, walls, signs, labels, the hand menu, the pill, rays and rings stay stylised and emissive, under the shared haze (`rendering.md`). They are the interface, and they stay legible against any scenery because they give their own light. Only what gains from it changes: the controllers, a real object in the hand and already textured glTF, can take PBR. Open question 5, whether the swarm is lit by the environment, stands.

### What it means for each environment

| The farmyard's | Today | High quality |
| --- | --- | --- |
| Ground: the yard's packed earth, the meadow and pasture, the trail, the terrace's moss and stone, the beach's shingle | flat faces, each colour varied a little | the same facets, textured with generated tiling sets of earth, grass, gravel, moss and shingle, blended by a mask in vertex colour and by slope in the shader, with detail normal maps; darker and glossier where wet |
| Plants: the bush, pines, tree ferns, flax and ferns | low-poly leaves, coloured on both sides | the same low-poly leaves and fronds, with generated colour and vein variation; two-sided, with a cheap transmission term so light from behind shows through; generated bark; the breeze kept. Cut-out cards, whose overdraw a tiled GPU pays in full, only where the census's layers over each pixel allow |
| Wood: the fence, the coop, the workbench, the ramp, the logs | flat colour | generated grain on the fence, ramp and logs: grey, rough posts. The coop and the workbench authored as hero objects: the coop's red paint and white trim worn through to wood at the edges; the bench oiled and smoother |
| Water: the river, the pool, the waterfall | a custom pipeline | still custom, now physically based: Fresnel reflection of the environment map, ripples made in the shader on a generated normal map, colour absorbed by depth taken from the bed's geometry rather than read from the depth buffer, and foam where the fall lands. Ray-traced or planar reflection in Ultra |
| Stone: the river's stones, the fire ring, the waterfall rock and cliff | flat faces | generated base colour, normal and roughness; darker and smoother where wet |
| Fire, embers, lanterns, marshmallows | emissive | emissive as now, flickering in the shader, their light baked as layers, with bloom in Ultra |
| The hens | their author's model, by Hinoki Maple Williams under CC BY 4.0 | as their author made them, with physically based textures if their author adds them |

**Sky:** its platform and shrine take stone and wood materials, and its night sky becomes its environment map, with the moon as its direct light. Its build-time bake of occlusion and lantern light becomes lightmaps in stage 10, and stays as it is until then. The moon's shadow map stays, and the volumetric clouds stay a custom pipeline. **Environments still to come**, such as a data centre, start physically based.

### Where textures and materials come from

**Decided: a hybrid of three sources,** each for what it suits.

| Source | For | How |
| --- | --- | --- |
| **Generated offline, by code** | tiling surface materials: ground, grass, gravel, stone, bark, wood grain, and normal maps for water and rock | A generator in the repository, from fixed seeds, makes each set's base colour, normal and ORM, tiling without seams, as the farmyard's geometry is made in code. Its output is compressed offline into KTX2 with mips, ASTC for the Quest and BC7 for desktops without ASTC, and shipped from the content store by hash. Changing a material is a change to code, reviewed as code; the same seed gives the same bytes, so every build and the server's baker agree |
| **Authored offline** | distinctive hero objects, and anything looked at up close: the coop, the workbench, the hens, an environment's own signs and plaques (workers' signs stay Tomori's, stylised) | Made in the asset studio or Blender, as models with painted UV maps, exported as `.glb` with their textures. Each records its provenance and licence, listed with a build's licences, as the hens' credit already is: by Hinoki Maple Williams, under CC BY 4.0 |
| **Procedural on the device** | detail and animation with no stored texture: water's ripples, embers' flicker, variation that breaks up tiling, blending by slope | Made in the material's shader from its parameters, at each tier's fidelity (section 8) |

- **A size budget for each environment:** proposed at 32 MB of textures shipped, all tiers together, and 64 MiB resident on the Quest (below). Each environment's tests check it, as they check their triangle budgets.
- **The reader:** `tomori-assets` gains glTF's normal, occlusion, metallic-roughness and emissive textures, a second set of texture coordinates (`TEXCOORD_1`) for lightmaps, tangents made with MikkTSpace where a file has none, and KTX2. Authored files with Basis Universal textures are transcoded at load, as the controllers' are now; that moves from `tomori-render` into `tomori-assets`, under its reader's limits for untrusted input.
- **Delivery:** embedded in the build and the APK, as models are now (`assets.md`), until the content store's stage 1. After that, textures are served by hash as `image/ktx2`, with a size bound like the store's other kinds, fetched as an environment is entered, smallest mips first; each surface is drawn with its constant factors until its textures arrive.

### The Quest's budget

- **Texture memory:** proposed at 64 MiB of textures resident for each environment. Base colour and ORM use ASTC 6×6, 3.56 bits a texel; normals use ASTC 4×4, 8 bits a texel. One 1,024² set of all three, with mips, is then about 2.7 MB, where uncompressed it would be 17 MB, so twenty such sets come to about 53 MB. The reader keeps its limit of 2,048 texels a side.
- **Mipmaps everywhere,** filtered trilinearly. Besides removing shimmer, they cut the memory bandwidth of distant pixels, which is the dearest thing on a tiled GPU.
- **Shader cost per pixel:** the PBR shader is estimated at two to four times the stylised one's, which has not yet been measured. At the recommended size with 4× MSAA the eyes took 6.72 ms of the GPU's 13.9 ms frame at foveation high; at #527's twice the size with 2× they took 10.75 ms on average in Observe, 11.45 ms at the 95th percentile. So the headroom #527 spent on sharper text is what High's PBR must share. Each of these offsets part of it:
  - **Foveation** already took the eyes from 8.32 ms to 6.72 ms, 19%. Towards the edges of each eye it shades one fragment for every two to sixteen pixels, so most of PBR's cost per fragment is spent where each eye looks straight ahead.
  - **Each tier's resolution** (section 8): High may draw at less than twice the recommended size where its materials need the time, a setting the maintainer chooses by eye, as #527's was.
  - **Light masks** from stage 3 keep each pixel's loop to its own lamps, at most 8.
  - **Baked light** stands in for real-time indirect light.
  - **Cheaper arithmetic:** half precision where it suffices; one ORM fetch for three parameters; and a cheaper variant, with fewer texture reads, for distant pixels that the haze mostly hides.

  If a stage misses its gate, texture reads and precision are cut before features are.
- **The gate for each stage**, measured on the Quest 3 at 72 Hz in High, at its default resolution and samples with foveation at high, from the census's viewpoints:
  - the eyes' GPU time at most 11.5 ms at the 95th percentile, the rule by which `rendering.md` admits a device's defaults (the 9.5 ms on average first proposed here predates #527, whose sharper eyes already take 10.75 ms in Observe);
  - no more missed refreshes than before the stage;
  - the runtime's GPU level no higher than it chose before stage 6.

  Low and Ultra have gates of their own (section 8).

### What the Quest 3 can do

At 72 to 90 Hz in stereo, at 1,680 × 1,760 for each eye with 4× MSAA, the Quest 3 can do what shipping Quest titles generally do:

- physically based materials;
- a handful of real-time lights on each object;
- baked lighting with light probes and reflection probes;
- fixed foveation;
- no post-processing chain.

These are out of reach there:

- real-time global illumination;
- screen-space reflections or ambient occlusion;
- volumetric lighting over the whole scene;
- anything like Nanite or Lumen, which Unreal itself does not offer for Quest.

The desktop window and the spectator have one view and a desktop GPU many times the Quest's. They can add HDR rendering with bloom, cascaded shadow maps, SSAO and, on hardware with ray tracing, real-time global illumination; Bevy's experimental Solari shows that path on wgpu.

### The options it was chosen from

1. **Stylised only.** Today's look with more lights, light lists and server-baked per-face light: warm bounce from the fire, soft occlusion under the coop. The cheapest; no new asset pipeline; the look the maintainer loves, deepened.
2. **Stylised PBR.** Add the `Pbr` shading model beside `Stylised`, chosen by each material.
   - It brings glTF metallic-roughness, normal maps, image-based light from a small prefiltered environment map (a 128² cube map and its irradiance), and tone mapping in each shader.
   - Faceted models stay faceted, with PBR's specular and Fresnel on them. Imported models from asset libraries look as their authors meant.
   - Light is baked from the server, with probes for what moves.
   - It runs on the Quest within the budget if fixed foveation lands. The PBR shader costs perhaps two to four times the stylised one for each pixel, an estimate stage 1's profiler will check, and the headset's GPU is three-quarters busy now.
   - It is what Quest titles that look realistic do.
3. **Realistic on the desktop, stylised PBR on the Quest.** The same scenes and materials. The desktop and spectator add an HDR target, post-processing, real-time shadows and optionally ray-traced GI.
   - The spectator and recordings then look their best; the headset looks like option 2.
   - The cost is two quality tiers to keep and test, and scenes that look different between devices, which matters when a participant on the desktop and one in a headset talk about the same thing.
4. **Fully realistic everywhere.** Not achievable on the Quest at 72 to 90 Hz in stereo today. Choosing it would mean streaming frames rendered elsewhere to the headset, a different product.

The proposal had recommended building for options 2 and 3, keeping option 1 for the farmyard and Sky. The plan reads the decisions as option 3 for every environment, with option 2's faceted models kept faceted and textured, and option 3's two tiers grown into section 8's three, with Low for weaker devices. The shading model still belongs to each material, and the lighting setup to each environment, so Tomori's own stylised things and physically based scenery share one frame.

## 8. Render tiers: one scene, several paths

The maintainer asked that a weaker renderer fall back to static or cheaper lights and coarser shapes, perhaps through LOD, "ideally in a way that supports different rendering pipelines, e.g. high quality vs low quality vs ray traced. Open to ideas." This is the proposal for it.

### One scene description, interchangeable render paths

- **The scene is described once, in the render world.** Environments and Tomori's own things submit instances, meshes, materials, lights, environment light and bakes to `tomori-world`, as they do now, and never learn which path draws them. Culling, light assignment, sorting and LOD selection stay in the world, shared by every tier.
- **A render path draws it.** `tomori-render` gains a `RenderPath` for each tier. Each takes the world's prepared, culled and sorted lists and encodes the eyes' passes. A path chooses its pipelines for each shading model, its light budget, its shadows and indirect light, its LOD bias, its resolution and samples, and any passes of its own. Only Ultra adds passes; Low and High keep section 5's single pass.
- **Tomori's own things are drawn alike in every tier.** The panels, walls, text, signs, swarm, rays and labels keep `rendering.md`'s contracts in every path, by the same pipelines. Only the scenery's look changes with the tier.
- **Materials are authored once.** A material in `tomori-world` holds:
  - its shading model;
  - its constant factors;
  - its texture slots;
  - its procedural parameters (ripple, flicker, anti-tiling, slope blend);
  - its alpha mode and sidedness;
  - a flat colour, the mean of its base colour, computed offline.

  Each path reads what it can afford. Low may draw a ground material by its flat colour or its smallest mips; High draws every slot; Ultra adds ray-traced reflection by the same roughness. An environment's custom pipeline, such as the water, the fire, the ferns or Sky's clouds, gives a variant for each tier it cares about, and is drawn by its one pipeline in the rest.

| | **Low** | **High** | **Ultra** |
| --- | --- | --- | --- |
| For | weaker headsets (a Quest 3S, a Quest 2), a web client, a device over its budget | the Quest 3, the target | the desktop window and the spectator on a desktop GPU |
| Lights | baked or static light where a bake exists, with each switchable layer still scaled live; otherwise today's sky and ground ambient. At most 2 real-time lights an item | baked light and probes, the sky's ambient and image-based light, at most 8 real-time lights an item (stage 3) | as High, with more lights by clustered forward |
| Materials | the stylised shading model over the faceted shapes: base colour from the flat colour or the smallest mips, no normal or ORM reads | `Pbr`, every texture slot, procedural detail in the shader | as High |
| Shadows | baked, and a blob under what moves | baked, and a blob under what moves | ray-traced or hybrid where the GPU has ray queries, cascaded shadow maps otherwise |
| Reflections and GI | none beyond the sky's ambient | the prefiltered environment map, reflection probes | ray-traced reflections, or screen-space or planar ones, and ray-traced GI where the GPU allows |
| Geometry | coarser LOD levels, switched nearer | the levels as built, at a pixel of error | the finest levels, switched farther |
| Resolution and samples | the recommended size, 4× or 2× | the device's measured default, on a Quest 3 twice the size with 2× (#527) | the window's size, an HDR target with bloom and tone mapping |

- **Ray tracing through wgpu.** wgpu 30's ray queries (`Features::EXPERIMENTAL_RAY_QUERY`) are experimental and on Vulkan alone, so Ultra asks for them where the adapter has them and falls back to shadow maps and screen-space reflection where it does not, as on Metal. The server's baker can use the same queries (section 6).
- **LOD as part of the tier,** generic, as #484's review asked. An item may name coarser meshes of the same model, built in code with the model, as procedural models are, or at build time for authored ones, never at startup. The world chooses one level for both eyes by pixels of error, with hysteresis and no fade, measuring ferns from their heart. #484's selection, `Levels` with its hysteresis, can merge with this consumer. Each tier sets the bias, from Ultra's finest to Low's coarsest. The faceted style makes a coarser level the same model with fewer facets.

### How a device picks its tier

1. **Its capabilities, at startup.** A desktop adapter that is not a tiled GPU, with ray queries, takes Ultra; without them, Ultra's fallbacks. A headset takes High only where its model has been measured holding its frames there, as `xr_types::MEASURED` admits a Quest 3's eyes by default today. Any other headset, or a device short of a feature High needs (such as ASTC or BC7 textures), takes Low.
2. **A setting** overrides it: `-e tomori.tier low|high|ultra` on the headset, `TOMORI_TIER` for `tomori --xr`, and the desktop's and spectator's options. The log says at startup which rule chose the tier, as it does for the eyes' size.
3. **Measured frame time.** If the eyes' GPU time stays over the frame's budget, 11.5 ms at the 95th percentile at 72 Hz, for 10 seconds, the device steps down: first its resolution, then one tier. Each step is logged, and none is taken back within the session, so that it never oscillates. A pinned setting turns this off. Switching changes pipelines and the textures' resident mips, not the scene.

### How the tiers are tested

- **Pixels for each tier.** The render tests draw the census's viewpoints on lavapipe in each tier and check each tier's look on its own: surfaces within their material's range of luminance, and LOD changing pixels only where a level switches. Across tiers, they check what must agree:
  - culling and order are the same;
  - Tomori's own things are the same pixel for pixel;
  - each surface's mean luminance is within a set fraction between tiers, so that a desktop and a headset look like the same place.

  Ray-traced paths are tested on lavapipe where it offers ray queries, and their fallback always.
- **Frame budgets for each tier,** on its reference device, by the profiler's `GPU over` line:
  - Low: within 11.5 ms at the 95th percentile on a Quest 3S at 72 Hz, and well under High on a Quest 3;
  - High: section 7's gate on a Quest 3;
  - Ultra: within the window's refresh at its size, timed by `scenery_frames` with each extra on and off.
- **The census for each tier:** triangles in view after LOD, and lights for each item, within the tier's budgets, on CI without a GPU.

## 9. Bevy, compared

| Bevy (0.20) | Tomori today | What to borrow |
| --- | --- | --- |
| A main world and a render world. The **extract** schedule copies what rendering needs each frame, then *prepare*, *queue*, *sort* and *render*; rendering can run pipelined on its own thread | The frame step makes a `Frame`, and `scene_graphics::draw` applies it to the renderer; sceneries are opaque | The phases, and a retained render world keyed by stable handles so that only what changed is uploaded, as panels already are. A render thread is an open question: the headset's frame loop overlaps CPU and GPU already, and OpenXR's frame calls constrain where a second thread could start |
| `Aabb` for each mesh entity, `Frustum` for each view, `VisibleEntities` and `ViewVisibility` computed each frame, `NoFrustumCulling`, `VisibilityRange` for LOD with dithered cross-fades, `RenderLayers` | None | All of these, by name and meaning where they fit: bounds, the visible list for each view, the opt-out, visibility ranges, layers. For stereo, one combined frustum, which Bevy's XR support, the community's `bevy_mod_openxr`, does not give |
| Clustered forward lighting (`ClusterConfig`), lights assigned to froxels on the CPU, with a uniform-buffer fallback of limited lights for weaker platforms | Four lamps, every pixel | The data layout and assignment, when clusters are needed (stage 3 starts with nearest-N) |
| `StandardMaterial`: metallic-roughness PBR on Filament's model, normal maps, emissive, environment map light, irradiance volumes, light probes, reflection probes, lightmaps, tone mapping (AgX, TonyMcMapface and others), cascaded shadows; Solari, experimental ray-traced GI | Stylised lighting, `tomori-assets`' base colour and emissive | The BRDF and its approximations for mobile, the material's fields, which follow glTF, and the probes' representations. Tone mapping inline in Tomori's shaders, not a pass |
| Crates split by role since 0.17: `bevy_camera`, `bevy_light`, `bevy_mesh`, `bevy_material`, `bevy_shader` beside `bevy_render`, `bevy_pbr` and `bevy_core_pipeline` | One render crate | The split of the data and visibility, GPU-free, from the GPU backend: `tomori-world` beside `tomori-render` |
| Standalone helpers: `offset-allocator`, `bevy_mikktspace` | | These, directly, where useful: suballocating mesh and placement buffers; tangents for normal maps |

### Why not depend on Bevy's render crates

- **Versions are not the obstacle.** `bevy_render` 0.20.1 uses wgpu 30 and glam 0.33, the versions Tomori uses.
- **The ECS is.** `bevy_render`, `bevy_pbr` and `bevy_core_pipeline` depend on `bevy_app`, `bevy_ecs`, `bevy_asset`, `bevy_extract`, `bevy_transform` and more. Using them means running Bevy's app, schedules, ECS and asset server inside Tomori's frame loop: the reason `physics-and-stacking.md` turned Avian down.
- **The pipeline is desktop-shaped.** Its 3D pipeline renders into an HDR target, then tone-maps and post-processes in passes of their own, with optional prepasses. On the Quest each extra full-screen pass writes and reads every tile, which is what section 5 avoids.
- **Tomori's contracts are its own.** The order of translucent surfaces, the occluders written fragment by fragment, text as glyph meshes, one projection layer and the panels kept across frames are Tomori's, in `rendering.md`, and a general engine would fight them.
- **Bevy breaks its API about every three months.**

Using wgpu directly, as now, keeps Tomori's renderer small and its contracts its own, while its designs come from Bevy.

## 10. Profiling and budgets

- **A frame profiler, logged and shown.**
  - **GPU:** timestamp queries at the start and end of each eye's pass, and on the desktop, within passes, around each layer: sky, opaque scenery, Tomori's opaque, translucent panels and signs, labels.
  - **Counts:** draw calls, triangles submitted, items culled and drawn, and lights assigned, for each frame.
  - **Where it goes:** into the minute's `CPU over …` line as a `GPU …` line beside it. On the headset, the runtime's `App` time and levels are logged next to it from the same minute. A hand menu toggle shows the last second's figures on a panel, for the maintainer to read in the headset.
  - **On the Quest,** timestamps within a tiled pass are not meaningful, so its split by layer comes from a debug toggle that leaves layers out, as this proposal's measurements did, or from RenderDoc for Quest when a frame needs taking apart.
- **The census as a test.** `tomori-world`'s census runs each environment's static scene from the viewpoints it declares, the arrival, its stations and anchors, and asserts budgets: triangles and draw calls in view, lights for each item, and back-facing share for closed meshes. It needs no GPU, so it runs on CI, beside the triangle budgets the environments already keep.
- **A GPU benchmark,** like `panel_frames`, drawing each environment's scenery at the headset's size: run by hand and before releases, not a CI gate, since CI's lavapipe is not a GPU to time.

## 11. A staged plan

The plan as approved, before the decisions on realism and tiers:

| Stage | What | Expected wins, from the measurements | Size |
| --- | --- | --- | --- |
| **1. Bounds, culling, order and a profiler** | Bounds for models' batches, instances, ferns, Tomori's cubes and panels. Frustum culling against the combined frustum: batches, then instances, writing only visible placements. `Scenery` gains a `cull` hook with the frame's views. Back-face culling for materials marked closed. The sky after the opaque scene. GPU timestamps and counts in the frame log. The census as a CI test, starting with the farmyard's four viewpoints | Model triangles submitted fall to 41 to 70% facing, 55% over all headings; ferns to 26 to 44%; the scenery's model draws from 18 to 9 to 16. The sky's move, 12 to 16% of the scenery's GPU time on the desktop. Back faces about half of those in view, for closed meshes. The CPU's encoding falls with the draws. The headset's split of its 9.5 ms is known for the first time | M: 3 to 4 PRs |
| **2. The render world** | `tomori-world`: items, meshes, materials, lights and views; static meshes chunked; the BVH; sorting; Tomori's own things and the farmyard and Sky submitting to it; `Scenery::draw` kept as an escape hatch; the environment registry moved out of `Sceneries` | Triangles in view towards their own 14 to 44% facing, 27 to 29% over all headings. The CPU cost grows with what is visible rather than with all there is. The world the later stages build on | L: 4 to 6 PRs |
| **3. Light lists and a light budget** | Nearest-N with a budget of 64 a scene and 8 an item, culled by reach and frustum, fading at the cut; the shader looping over the item's list; `LAMPS` gone | Most of the 8 to 13% the four lamps cost now; more lamps in the farmyard, the trail, the coop, without each pixel paying for all of them | M: 2 PRs |
| **4. LOD and multiview** | Visibility ranges with dithered fades; the hens, bush and ferns given far meshes; multiview for every pipeline, sceneries' uniforms by view | The hens' 6,300 triangles to under 1,500 beyond 6 m; the CPU's encoding, 2.2 to 2.9 ms on the headset, about halved | M to L: 3 PRs |
| **5. Fixed foveation** | `XR_FB_foveation` with the runtime's fragment density map, through a patch to wgpu's Vulkan backend | Typically 10 to 25% of fragment work, the GPU's largest share; to be measured | M, risky |
| **6. PBR** | Now stages 6 to 9 below, after the decisions on realism and tiers | | |
| **7. Baked light from the server** | Now stage 10 below | | |
| **8. Desktop extras** | Now stage 11 below, Ultra | | |

Stage 1 was worth doing before the render world, since it paid at once and its census and profiler tell the later stages what each buys. The crate split came in stage 2, with the world itself.

### Where it stands, and the stages after the decisions

- **Built:**
  - stage 1, culling, the sky's order and the profiler (#380);
  - stage 3, each piece's own lamps (#475, #476);
  - stage 5, foveation (#405), which took the eyes from 8.32 ms to 6.72 ms.
- **Under way:** stage 2's render world (#411, #413) and chunks (#481). Sorting, the rest of Tomori's own things and the environment registry remain.
- **Tried and closed:** stage 4.
  - LOD (#484) returns as a generic part of stage 9.
  - Multiview (#414) gave way to resolution with 2× MSAA (#527).

The decisions in sections 7 and 8 ask for six more stages. Each ships on its own and must pass its gate before the next begins. High's gate is section 7's, on a Quest 3, measured by the profiler's `GPU over` line from the census's viewpoints. Low's and Ultra's gates are in section 8. Low is today's renderer from the start: when stage 8 makes High physically based, weaker devices keep today's look rather than losing support.

| Stage | What | Gate, and what it buys | Size |
| --- | --- | --- | --- |
| **6. Materials in the render world, and the tiers** | `tomori-world` describes materials as data, as section 8 lists. `tomori-render` gains `RenderPath`, with today's renderer as its first path, drawing both Low and High for now, Ultra falling back to it. The tier is chosen at startup by capabilities and setting, and logged. Each item's light budget is the tier's. `tomori-assets` reads glTF's full metallic-roughness material, `TEXCOORD_1`, tangents and KTX2, with a texture cache under a memory budget, smallest mips first. Normal maps reach the stylised shader, within each face | Today's render tests unchanged in every tier; the eyes' GPU time within 2% of before. The abstraction every later stage plugs into | M to L: 3 PRs |
| **7. The texture generator, and delivery** | Needs assets by digest (#194, in progress), so that textures are named by hash. The offline generator in the repository (section 7): tiling sets of earth, grass, gravel, stone, bark and wood grain, normal maps for water and rock, compressed to KTX2 with mips, ASTC and BC7. Its output's hashes are committed, and CI regenerates them to check that code and output agree. The bytes are embedded in the build until the content store's stage 1, then served from it by hash. Each environment's texture budget is checked in its tests. Faceted meshes have no UVs, so the stylised shader projects each texture planar from the face's own normal, per face, and the generated sets tile for it. Pilot: the farmyard's ground and fence, textured on its facets in today's shading | Tiling without seams, checked at each set's edges; the same bytes from the same seed; within 32 MB shipped and 64 MiB resident; the eyes within 5% of before. Detail on the faceted surfaces, visible before PBR | M: 2 to 3 PRs |
| **8. PBR shading in High** | A `Pbr` shading model on Filament's mobile BRDF: GGX, Schlick's Fresnel, split-sum image-based light from each environment's sky captured into a 128² prefiltered cube with irradiance as spherical harmonics, occlusion that also dims reflections, emissive, and exposure and tone mapping inline. Procedural detail on the device: water's ripples, embers' flicker, anti-tiling, slope blending. Hero objects authored, the coop and the workbench first, with provenance and licences. The farmyard's and Sky's scenery change over material by material: ground, wood and stone, plants, then water. Tomori's own things stay stylised | Section 7's gate at every viewpoint; each surface within its material's range of luminance. The look the maintainer asked for, on the Quest 3 | L: 3 to 4 PRs |
| **9. Low's fallbacks and generic LOD** | Low reads materials by their flat colour or smallest mips, at most 2 real-time lights an item, and static light where a bake exists. Generic LOD in `tomori-world` (section 8): coarser levels built in code with the farmyard's models, at build time for the hens, chosen by pixels of error with hysteresis, the tier's bias, ferns from their heart. Measured on a Quest 3S, which then joins `MEASURED` at its tier. A device over its budget steps down | Low within 11.5 ms at the 95th percentile on a Quest 3S at 72 Hz, and its eyes well under High's on a Quest 3; LOD changing pixels only where a level switches. Weaker devices supported, with coarser shapes where they don't show | M to L: 3 PRs |
| **10. Baked light** | `tomori-lighting`, as section 6 describes: lightmaps for each light layer, L1 probes for what moves, reflection probes for what is wet or shiny, and per-face irradiance for Low. Baked by the server's core, stored and served by hash. Until then a build-time bake, as Sky's is, can light the farmyard's static scenery | Lightmaps at most 4 MB an environment; sampling them at most 0.3 ms of the eyes' GPU time in High, less in Low. Bounce and occlusion in every tier, shared by everyone in the space | L: 3 to 4 PRs; needs the content store's stage 1 |
| **11. Ultra** | For the desktop and spectator: an HDR target with bloom; clustered forward lights; ray-traced or hybrid shadows and reflections by wgpu's ray queries where the adapter has them, with cascaded shadow maps and screen-space or planar reflection otherwise; ray-traced GI where the GPU allows. Reverse-Z with an infinite far plane is reconsidered for its shadow maps. Each extra can be turned off, and none changes the headset | Within the window's refresh, timed by `scenery_frames` at the window's size, each extra on and off; each surface's luminance within the set fraction of High's. Recordings and the spectator at their best | L: 3 to 4 PRs |

## Risks

- **The headset's GPU time is not yet divided.** The measurements say how much the farmyard's scenery costs on a desktop, and that pixels dominate there. They do not say how much of the Quest's 9.5 ms is the scenery, the panels, the walls or the swarm. If the walls' translucent boards dominate, culling the scenery will save less than its numbers suggest, and the panels' overdraw becomes the work. Stage 1's profiler answers this before stage 2 begins.
- **Downstream sceneries.** `Scenery` is public, and environments outside this repository implement it. The render world changes how they draw. `Scenery::draw` stays as an escape hatch for a release, and the change is announced in the release notes, by a `Breaking:` paragraph in its pull request's description.
- **The translucency contract.** Culling must not change which surface is in front. Only whole surfaces outside both eyes are dropped, and the order of the rest is the frame step's; `rendering.md`'s render tests check this.
- **Foveation through wgpu.** It wraps the device's Vulkan render pass and framebuffer functions, since wgpu does not expose density maps, so each wgpu upgrade must be checked on the device. Ultra's ray queries are experimental in wgpu and may change.
- **Three paths to keep.** Each tier is code to maintain and a look to test. The paths share the scene, culling, lights, sorting and Tomori's own surfaces, and differ only in pipelines and parameters; every tier is drawn by CI's render tests, with the cross-tier checks of section 8, so a change cannot silently break the tier nobody is wearing.
- **PBR's assets.** Textures cost memory and download, and need KTX2 and ASTC on the Quest. Imported models need tangents. The fragment cost must be held to a budget per environment, the gate in section 7. Cut-out foliage is the likeliest to miss it, since its overdraw is paid in full on a tiled GPU.
- **Baking.** It depends on the content store, not yet built. Bakes must be deterministic to be cached, and bounded in time on the server. Light must never visibly lag what someone just did.

## Open questions for the maintainer

**Decided, and moved into the text:**
- what realistic means (section 7);
- where textures come from, the hybrid of three sources (section 7);
- that a weaker renderer falls back to cheaper lights and shapes, over different pipelines (section 8, whose tiers and their names are this proposal's plan);
- how low-poly the shapes stay: faceted, as now (section 7);
- the crate split, `tomori-world` first, in stage 2 (approved earlier).

**Still open:**

1. **72 Hz or 90 Hz?** Should the savings buy 90 Hz, or stay at 72 Hz for cooler, longer sessions and more content? Section 8's budgets assume 72 Hz.
2. **Light budgets:** Low's 2 an item, High's 8 an item and 64 a scene, or another balance?
3. **Low's reference device:** a Quest 3S, as stage 9 assumes, or a Quest 2? And should a web client be a target at all?
4. **Stepping down by itself:** should a device over its budget drop a tier during a session, as section 8 proposes, or change only at startup and by setting?
5. **Should Tomori's own things be lit by the environment?** The swarm and its cubes are lit by a fixed light of their own and hazed with the scene, not lit by the lanterns or the moon. Lit by the environment, the agent would glow warm by the campfire, but might lose the clarity that its own light gives it.
6. **Baking:** server-side for environments that change, and at build time, as Sky does, for those that don't?

## Design changes this would make

As each stage is built:

- `rendering.md` gains visibility, the light budget, the sky's order, foveation, the profiler's log, and the render tiers: how a device picks one, and what each must keep;
- `environments.md` gains what a scenery submits;
- `crates.md` gains `tomori-world` and, later, `tomori-lighting`;
- `clients.md` gains the environment registry's move to `tomori-scene`;
- `assets.md` gains PBR materials, their textures, KTX2, LOD levels, the texture generator and each texture's provenance and licence;
- `farmyard.md` gains its viewpoints' budgets, and its look changes from flat faces to textured, physically based materials;
- `channels-and-content.md`'s content kinds gain baked light and KTX2 textures.

Until each is built, this document is a proposal, not a requirement.
