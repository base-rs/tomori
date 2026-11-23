# Proposal: Panels as Compositor Layers

Status: **parked**, November 2026. Measured on the maintainer's Quest 3 and not worth building now: the GPU time it could save would come only from drawing the 3D scene at a lower scale, and that looks clearly worse. 2.0× with 2× MSAA fits the frame. This document records the idea, the measurements and the open questions, so that it can be picked up again.

It builds on the eyes' resolution and samples (#527), the one composition layer `design/rendering.md` requires (**One composition layer**), the panels' level of detail by angle (#506) and the glyph atlas in `design/text.md`.

## The idea

Tomori is text-heavy: walls of panels with text, lists, tables and charts, drawn into the eye buffer with distance-field text. Text sharpens with resolution. MSAA does not sharpen it, because a panel's text and lines are made in the fragment shader inside quads. Two ways to get sharper panels:

- **Draw the eye buffer at a higher scale**, as #527 does: 2.0× the runtime's recommended 1,680 × 1,760, with 2× MSAA.
- **Submit panels as OpenXR quad layers** (`XrCompositionLayerQuad`), each panel's face rendered at its own angular resolution into a swapchain of its own. The compositor samples a layer once, straight into the distorted display image, and may supersample and sharpen it (`XR_FB_composition_layer_settings`). Meta applies supersampling only to quad and cylinder layers, not to projection layers, which matches the `SP=N/N/N` the headset reported for the projection layer. The rest of the scene would then be drawn at about 1.0–1.3× with 4× MSAA.

## Measurements

Quest 3 GPU eye time in Observe's busy view, mean, from Tomori's `GPU over` line. The runs used a throwaway debug build with an intent extra, `tomori.debug_draw panels_off`, that left out every projected panel's face and every wall's board and drew everything else. That build is not part of this change.

| Eyes | Panels drawn | Panels off | Panels' cost |
| --- | --- | --- | --- |
| A: 2.0×, 2× MSAA (#527's default) | 10.75 ms | 10.14 ms | 0.61 ms |
| B: 1.0×, 4× MSAA | 7.80 ms | 5.26 ms | 2.54 ms |

As layers, the panels would still be rendered at full resolution, so what they cost stays about the same either way. The saving is the scene without panels at A less the same at B: **A0 − B0 = 4.9 ms**, minus what the layers cost.

The layers' cost has three parts:

- **Rendering each layer's texture.** This happens only when the panel's content, frame colour, scroll or readout changes, not every frame.
- **A flat compositor cost per layer.** Meta gives about 0.1 ms per layer on a Quest 2, and no figure for the Quest 3.
- **A compositor cost per pixel the layer covers.** Meta gives about 0.6 ms for a full-screen layer on a Quest 2.

For 8 to 15 layers that comes to perhaps 0.5–1.5 ms. That leaves roughly 3–4 ms saved, at a 72 Hz frame of 13.9 ms that A already fits.

Viewed in the headset, the maintainer found the 3D scene at 1.0×/4× clearly worse than at 2.0×/2×: geometry's clarity matters as much as the text's. Saving the time would mean accepting that, and nothing needs the time today. So the idea is parked.

Another oddity: panels cost four times as much at B as at A, though B has a quarter of A's pixels. 4× MSAA on panels' translucent, blended quads is a likely cause, but this was not investigated.

## When to revisit

- **At 90 Hz or 120 Hz**, where the frame is 11.1 ms or 8.3 ms and A's 10.75 ms no longer fits.
- **With much heavier scenes or GPU-bound environments**, such as realistic materials or a data centre's racks (see `rendering-architecture.md`), where the scene's own cost at 2.0× crowds the frame.
- **If the headset's display or runtime changes** so that a lower eye-buffer scale looks good enough for geometry, for example with the compositor's sharpening or super resolution applied to the projection layer.

Measure again first: A0 − B0 at the new rate or scene, against layers' cost measured on the Quest 3 itself, with `adb logcat -s VrApi`'s `LCnt` and the compositor's time.

## Open design questions, in brief

- **Occlusion and order.** The compositor draws layers in order and ignores depth, so a quad layer over the projection layer would cover hands, rays, controllers, the hand menu and avatars in front of it. There are two choices:
  - **Punch-through (underlay).** The layer is submitted before the projection layer. The eye buffer draws the panel's backing as a hole of alpha 0, depth tested after the opaque scene, and the projection layer is submitted with `XR_COMPOSITION_LAYER_BLEND_TEXTURE_SOURCE_ALPHA_BIT`. This is likely the better fit:
    - Anything nearer, opaque or translucent, covers the hole correctly. Premultiplied blending over a hole of (0, 0, 0, 0) gives exactly what the compositor expects.
    - A panel behind a 3D object is hidden by depth.
    - The costs: a quad per layer per eye, and an alpha channel every pipeline must keep correct.
    - The artefacts: the hole's edge is at eye-buffer resolution, and on stale frames the projection layer, reprojected without depth, slips against the layer, which is exact. Tomori submits no depth.
    - Panels' backings are 94–95% opaque, so a hole that multiplies the eye buffer by the backing's transparency matches today's look to within about 6% of the background under glyphs.
  - **Depth-tested layers** (`XR_FB_composition_layer_depth_test` with `XR_KHR_composition_layer_depth`). These would need the eye buffer's depth stored and submitted, which costs bandwidth on a tiled GPU. Translucent surfaces in front of a panel that write no depth, such as the label on the ray, hints and glows, would be hidden behind the layer.
- **Layer budget.** Meta Quest composites at most 16 layers, the projection layer included, and Unity's OVROverlay allows 15. The candidates are the K panels largest in angle, gazed at or near, with hysteresis, K perhaps 8. Carried, gliding or docking panels would stay in the eye buffer. A flat wall's slots could share one quad layer to cover more panels with fewer layers.
- **Switching.** A panel's placement is identical either way, so it can swap in one frame: the layer is rendered a frame ahead, then submitted in the same `xrEndFrame` that stops drawing the panel in the eye buffer. Alternatively it can cross-fade, using the hole's strength and the layer's alpha or colour scale (`XR_KHR_composition_layer_color_scale_bias`, which could also carry the haze). The change in sharpness is still visible, so selection needs hysteresis.
- **Texture size and memory.** Size each axis from the panel's projected angle times the display's roughly 25 pixels a degree, in a few fixed buckets with hysteresis so that swapchains are not remade. Each swapchain has about three images, so 8 layers at around 1024 × 640 come to about 60 MB. Only changed panels are re-rendered, using the faces' existing change tracking. `XR_FB_swapchain_update_state` sets the compositor's sampler (filters, mipmaps, anisotropy); it does no change tracking.
- **Interaction.** Hit-tests are the scene's, on the CPU, and do not change. Highlights, scrolling, a chart's readout and up to 8 linked readouts would be drawn into the layer and re-render it as they change, which while scanning a chart could be every frame for several layers. Alternatively they could be drawn over the hole in the eye buffer, at its lower sharpness.
- **Curved walls.** A curved wall's slots are flat, each with its own frame, so per-slot quads match today's drawing and hit-tests. Cylinder layers would bend the panels.
- **The desktop window and the spectator.** They have no compositor, so they keep today's path. Choosing layers would be the Quest client's alone, drawing the same face mesh with the same shader into each layer.

## Scope

Nothing changes. If this is taken up, it would revise `design/rendering.md`'s **One composition layer** requirement, and touch `crates/client-quest`'s frame loop and `crates/render`'s drawing of a panel's face into a target of its own.
