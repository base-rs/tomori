# Desktop Participation

This describes taking part in a server's spaces from a desktop window, without a headset or an OpenXR runtime.

## Requirement

**Taking part from a window.** `tomori --desktop` opens a desktop window that connects to a Tomori server and does what the headset does: arrival (`arrival.md`), visiting and knocking, pairing and admission (`security.md`), joining a space, its panel and controls (`interaction.md`), the hand menu (`hand-menu.md`), push-to-talk and spoken replies (`speech.md`, `live-audio.md`). It takes `--server`, `--name` and `--token` as the other participant modes do, and without `--server` lists the servers it discovers. The window without `--desktop` remains the server-less demonstration.

**The same behaviour, from the keyboard and mouse.** The window is the headset's frame loop with the keyboard and mouse standing in for its controls, not a separate interface: each control in `interaction.md` has a key with the same meaning and the same safeguards, such as holding B for a second to leave a space, and the rules that cancel held controls on losing focus apply when the window loses focus. The participant stands at arrival's home at standing height and turns their head by dragging with the right mouse button. The mouse points as the right controller's ray does, and its left button is the trigger. The hand menu opens and closes with a key, held in front of the participant as a raised left palm would show it, and is chosen in with the mouse. A waiting knock, which a headset cues on the wrist, is also named in the window's title. A participant's presence reports their head only; a desktop has no controllers or hands to report.

**Engagement.** A focused window is engaged, a visible but unfocused window is distracted, and a hidden window is away, as `spaces.md` derives from the client's focus; a desktop has no sensor for being worn.

**Anti-aliasing.** Desktop windows, including the server-less window and the spectator's window, draw their scene with 4× multisample anti-aliasing where the adapter supports it for the window's format and for depth, and with a single sample otherwise. The headset is unchanged, drawing a single sample, until its frame budget is measured with multisampling.

## Rationale

The user asked for a friend without a headset to try Tomori from the desktop client, admitted by knocking, in November 2026; until then only the terminal console and an OpenXR runtime could connect from a desktop, and the window could not. Reusing the headset's frame loop keeps one behaviour across devices, as `clients.md` requires. The user asked for anti-aliasing on the desktop display on first seeing the window. Multisampling is applied only to the scene pass: offscreen passes, such as the cloud integration and the panel's text, keep their own targets.

## Scope

`crates/client-quest`: the `--desktop` mode (`desktop_participant.rs`), the shared desktop window setup (`desktop.rs`), and the renderer's sample count (`graphics.rs` and every pipeline drawn in its scene pass, including `crates/environment-sky`).

## Validation

- Unit tests cover the participant's head: it starts at arrival's home facing arrival, the window's centre points where it looks, looking is clamped short of straight up and down, and the hand menu's palm is in view and faces the eye.
- The ignored Vulkan test `multisampling_smooths_edges_in_both_environments` renders the scene pass's pipelines with one and four samples in each environment, without validation errors, and checks that four samples blend edges; set `TOMORI_MULTISAMPLE_PREVIEW=PATH` to save the comparison.
- Manually, run `tomori --desktop --server HOST:PORT` against a server with tokens: as a visitor, join a space, knock, and be admitted from another device; then talk with Space held, and confirm that the panel, the hand menu, and leaving with a held Backspace behave as on the headset.
