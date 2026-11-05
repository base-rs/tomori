# Clients

This describes the layout of Tomori clients. The headset and desktop application is `crates/client-quest`; it connects to the new server, and no longer uses `bridge/`.

## Requirement

Each kind of hardware or output has its own client implementation in a separate workspace crate in `crates/client-<target>`, named `tomori-client-<target>` as `crates.md` requires, such as `crates/client-quest` (`tomori-client-quest`) for Meta Quest headsets, `crates/client-desktop` (`tomori-client-desktop`) for the desktop window, console and discovery, and `crates/client-spectator` (`tomori-client-spectator`, whose executable is `tomori-spectator`) for the Linux spectator/OBS output. The `tomori` executable runs the desktop and spectator modes through those crates. Clients share a platform-neutral core:

- `crates/client` (`tomori-client`): the server connection, reconnection, outgoing audio, per-source jitter buffers, the playback mixer and the player that feeds it received audio for a device callback. Its `connection::Connection` is a client's whole connection: it carries out a `connection::Session`, the client's conversation with the server, choosing the server, keeping each server's certificate and token for the device, browsing for servers, recording from the microphone and playing through the speaker, behind the `capture::Capture` and `speaker::Playback` traits that each client's devices implement, and cueing what happens through the feedback model. It contains no rendering, windowing or device code, builds for every client target, and depends on no other client crate.
- `crates/scene` (`tomori-scene`): the shared scene model derived from space state and participant presence, with the frame step that the headset and the desktop window share, and the conversation, also free of rendering, windowing and device code, so that clients with different renderers present the same scene. It builds for every client target, and its test `tests/dependencies.rs` fails if it comes to depend on such a crate.
- The conversation is the connection's session: it implements `tomori_client::connection::Session`, so the dependency points from the scene to the client and never back. The session asks for `Action`s, which the connection takes one at a time; it is told the client's `Event`s and the connection's own `Report`s in order, whether to browse is asked of it, and once a frame it tells the feedback model what waits for the participant. Where each voice is placed, from the space's seats, is the scene's, given to the connection through `Connection::listen`.
- Engagement, whether the participant is wearing the device and attending, is derived in the core from platform-neutral signals (whether the application has input, and whether a sensor reports the device worn) by one state machine free of rendering and platform code. Each client feeds it its platform's signals, such as the OpenXR session state and the user presence extension on Quest, and the protocol carries only the resulting device-neutral fact; see `spaces.md`.
- Rendering code shared by clients that use the same engine lives in its own crate, not in one client that others depend on: `crates/render` (`tomori-render`) holds the wgpu renderers and shaders that the headset, the desktop window and the spectator share.

The protocol stays device-neutral. A client reports optional capabilities, such as hand tracking or controllers, through its messages; the server never branches on which hardware a participant uses.

The root manifest only defines the workspace and its build profiles; it has no package.

## Rationale

Tomori should support several kinds of hardware and outputs, each with its own platform constraints, without duplicating connection, audio or scene logic, and without one client's platform dependencies leaking into another. Agreed in the October 2026 design discussion.

The connection lives in `tomori-client` and the conversation in `tomori-scene`, at the maintainer's decision on #132 in November 2026. The `Session` trait between them lets another project drive Tomori's connection, with its pinning, pairing, reconnection, microphone and speaker, from a session of its own, and lets the connection be tested against a scripted session without the scene. It was drawn before the first release to crates.io, so that `tomori-client`'s interface does not change straight after it.

A visionOS client is a possible later target. Bevy and OpenXR are unavailable there, so it would render natively and use the core through a generated Swift binding. It is not planned, and the core is not required to provide that binding yet.

## Scope

Workspace layout and dependencies of client crates, the client core, the shared scene model, and the protocol's treatment of device differences.

## Validation

- Inspect the workspace: each client is a separate crate in `crates/client-<target>`, and the core and scene crates depend on no rendering, windowing or platform audio crate. `tomori-client` depends on no other client crate; Cargo refuses the cycle that a dependency on `tomori-scene` would make.
- `tomori-client`'s connection tests drive it through `Session` alone, with a scripted session, a scripted server and a real one, and `tomori-scene`'s tests check that the conversation applies each `Report`.
- Check that the core builds for every client target, including `aarch64-linux-android`.
- Inspect the server for logic that depends on a client's hardware rather than its reported capabilities.

Whether the spectator client mixes audio itself, instead of the server mixing the spectator feed as `live-audio.md` currently requires, is a pending follow-up.
