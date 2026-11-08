# Spectator View

## Requirement

Spectator output shows the scene without a head-locked HUD, including status captions and recording indicators, and plays no sound cues (see `feedback.md`). This applies to windowed and PipeWire output; the headset retains its own HUD. Connection and follow status may still be logged.

For the virtual camera, the PipeWire spectator renders at 1920×1080 at up to 30 fps, matching the virtual camera's 1920×1080 output rather than rendering at 4K and scaling down.

The spectator animates its swarm from the joined space's observed activity rather than displaying a static neutral arrangement, and shows it speaking while the agent speech it plays for its audience does, as the headset does (see `speech.md`). Worker transitions use the animated swarm's positions. This does not replace the shared-scene requirement in [clients.md](clients.md).

The spectator plays stereo and hears as the participant it follows does, from their head as their presence gives it in the space's shared scene, with each other participant at their head as their presence gives it, without seats, or from its neutral view with nobody to follow (see `spatial-audio.md`), so that its audience hears the agent where they see the swarm and each person where they are, wherever they have travelled.

The spectator follows the participant's head from their presence, which carries where they travelled in an environment that allows it (see `locomotion.md`), so it sees from where they are; it draws none of their teleport preview.

The spectator shows the environment of the space it watches, as everyone in it does, and follows it when an owner changes it (see `environments.md`). It stands the swarm at the space's agent anchor and the workers' arrangement at its workers anchor, and places workers with the placement registered with the sceneries it is given, or the default arc, by the same rule as the headset, so that both draw the coordinator and workers in the same places. Two things stay at the swarm's default place, an accepted limit: the neutral view it shows with nobody to follow, which looks at the default place from home, and the place its audience hears the agent from when the swarm has no agents to take the middle of, which it always has.

The spectator reports no engagement, so the server keeps it at the live edge of agent speech: its audience hears replies as they are spoken, never a catch-up of what was said before it joined. See `speech.md`.

## Rationale

The spectator view is intended for an audience and recording, where a headset HUD obstructs the scene and a static swarm misrepresents the session. The user identified both problems in November 2026 and requested a matching 1080p render and virtual camera for huddles.

Currently the headset's particle positions and controller attraction are not transmitted. The spectator runs the same activity-driven simulation locally, so it is not an exact copy of the headset's scene. This is a limitation to resolve as the shared scene model develops, not a requirement for the simulations to remain independent.

## Scope

Spectator rendering and its shared view, not headset rendering or in-world scene content.

## Validation

- Inspect the spectator render path to confirm it does not draw the caption or recording indicator; confirm that the headset continues to draw its HUD.
- View a connected spectator in a window or OBS and confirm that the scene is unobstructed while the headset HUD remains visible.
- Test that observed activity drives a moving spectator swarm using the same simulation as the headset, and inspect that worker transitions use its current positions.
- Check the PipeWire and virtual camera formats and verify that the camera receives frames at 1920×1080.
- Test that the spectator's conversation never reports engagement.
- Test that, after the followed participant travels and turns, the spectator hears from their head and places another participant at that participant's head from presence.
