# Plan: Splitting the Headset Crate, and the Headset as a Library

## Status

**Done, and released in [v0.2.0](https://github.com/ioquatix/tomori/releases/tag/v0.2.0).** November 2026. The binding requirements are in `design/clients.md`, `design/crates.md`, `design/builders.md` and `design/android-release.md`, which take precedence over this document. The maintainer's decisions on the open questions are recorded under [Decisions](#decisions), and what the first phase changed from this plan under [Progress](#progress). The rest of this document is the plan from [PR #132](https://github.com/ioquatix/tomori/pull/132), kept for its reasoning.

**Built:**

- **Phase 1, steps 1–5:** housekeeping ([#138](https://github.com/ioquatix/tomori/pull/138)); the rename to `tomori-client-quest` ([#139](https://github.com/ioquatix/tomori/pull/139)); `tomori-scene` ([#144](https://github.com/ioquatix/tomori/pull/144)); `tomori-render` ([#150](https://github.com/ioquatix/tomori/pull/150)); `tomori-client-desktop` and `tomori-client-spectator`, with a `tomori-spectator` executable ([#160](https://github.com/ioquatix/tomori/pull/160)); the scenery contract, the builder and `examples/headset` ([#161](https://github.com/ioquatix/tomori/pull/161)); and the connection moved into `tomori-client` behind a `Session` ([#174](https://github.com/ioquatix/tomori/pull/174)).
- **Phase 2, steps 6–8:** the application ID `com.codeotaku.tomori` and reusable Android packaging, with `examples/headset` packaged in CI ([#163](https://github.com/ioquatix/tomori/pull/163)); the client crates made ready to publish ([#164](https://github.com/ioquatix/tomori/pull/164)); and the signed headset APK attached to each GitHub Release by bake-cargo's release artifacts workflow ([#166](https://github.com/ioquatix/tomori/pull/166)).
- **The release:** v0.2.0 ([#178](https://github.com/ioquatix/tomori/pull/178)), with the crates on crates.io and `tomori-0.2.0.apk` on the GitHub Release.

**Changed from the plan, as decided later:**

- **Sky is published.** Decision 1 kept `tomori-environment-sky` private until #75, but `tomori-render` depends on it, so the maintainer decided on #132 to publish it too, which resolves the conflict that [Progress](#progress) describes.
- **The application ID** is `com.codeotaku.tomori` (decision 5), not `io.github.ioquatix.tomori` as the steps below say.
- **The release APK** is built unsigned from the release commit and signed on a runner of its own by bake-cargo's release artifacts workflow (`release-artifacts.yml`), rather than by an `apk` job in `publish.yml` as step 8 proposes.
- **`test_gpu`** is now in `tomori-render`, which Sky's tests use as a development dependency, rather than staying in Sky as Progress says.
- **No seats.** Progress says the connection hands the scene the other participants' heads "so that it places their voices on its seats". Since [#185](https://github.com/ioquatix/tomori/pull/185), voices are placed at each participant's head in the space's shared coordinates, with no seats.

**Open:**

- **Environments' client halves** in the builder wait for the shared environment interface, [#75](https://github.com/ioquatix/tomori/issues/75), as do Sky depending on `tomori-render` rather than through an adapter. The rest of #73's stage 3 is tracked there.
- [#56](https://github.com/ioquatix/tomori/issues/56), separating the spectator and the shared scene model from the Quest crate, is done by #144 and #160 but is still open.

This covers:

- [#56](https://github.com/ioquatix/tomori/issues/56): separate the spectator client and the shared scene model from the Quest crate.
- The headset half of [#73](https://github.com/ioquatix/tomori/issues/73) stage 3:
  - the headset as a library, with a builder that registers environments' client halves and sceneries;
  - reusable Android packaging;
  - the headset APK as a release artifact.

The server half of stage 3 (`tomori-server`'s builder) is planned and built separately. The headset's builder should follow its naming once it lands.

It builds on:

- the device boundary drawn in #119;
- the coverage decisions in #111, where only thin device files are excluded and each is listed with a reason;
- the environment framework proposal in #90 (`context/environments.md`), whose crate names `tomori-scene` and `tomori-render` the maintainer has adopted.

## Where things stand

Everything below is from `origin/main` at `17f4f6b`, together with the open coverage PRs #130 and #131, which this plan assumes have landed first.

**One package does everything on the client side.** `crates/client-quest` (package `tomori-quest`, library and executable both `tomori`) holds about 46,000 lines across 60 modules:

- the Quest headset: OpenXR in `xr.rs` and `vulkan.rs`, Android in `android.rs`, and AAudio in `capture.rs` and `playback.rs`;
- the desktop window's modes:
  - the server-less demo (`desktop.rs`);
  - the participant, with `--desktop` (`desktop_participant.rs`);
  - the spectator, with `--spectator`;
- the headless PipeWire spectator (`headless_spectator.rs`, `pipewire_video.rs`);
- the terminal console (`console.rs`) and `tomori discover` (`discover.rs`);
- every renderer and shader (`graphics.rs`, `*_graphics.rs`, `ui_panel.rs` and the `.wgsl` files);
- the platform-neutral logic:
  - the frame step (`scene.rs`, `scene/headset.rs`, `scene/desktop.rs`);
  - the conversation and the connection (`conversation.rs`, `connection.rs`);
  - arrival, the hand menu, delegation, the transcript and the rest.

**The neutral logic is already mostly separate from the device code.** After #119 and #130, device calls sit in a few files:

- `xr.rs` and `vulkan.rs`: OpenXR and Vulkan;
- `desktop_window.rs`: winit;
- `desktop_audio.rs`: cpal;
- `child_process.rs`: the forked child;
- `android.rs`: JNI;
- the `android` modules of `capture.rs` and `playback.rs`: AAudio.

Rendering, however, is mixed into otherwise neutral files:

- `ui_panel.rs` holds both the `Canvas`, which is pure layout, and the wgpu `Pipeline` and `Panel`.
- `arrival_graphics.rs` holds geometry and constants (such as `Beam` and `RAY_LENGTH`, used by the frame step) before its wgpu pipeline, which starts at line 368.
- `workspace_graphics.rs` holds `CardNavigation`, used by the frame step, beside its wgpu code.
- `controller.rs` holds the controller `Pose`, used everywhere, beside GLB and KTX2 decoding.
- `scene.rs`'s `Frame` is applied to the renderer by `scene_graphics.rs`.

**Some dependencies run the wrong way for a split:**

- `conversation.rs` and `connection.rs` depend on each other.
- `connection.rs` depends on the `Capture` and `Playback` traits, whose files also hold the AAudio code.
- `desktop_window.rs` constructs the spectator's and the participant's window contents itself.
- `spectator_feed.rs` opens the speaker through `desktop_audio.rs`.
- `test_support.rs`, the TLS test server and fake devices from #130, is used by tests that will end up in several crates.

**Environments are a closed enum.** `environment.rs` has `Environment::{Minimal, Sky}`. `graphics.rs` holds a `tomori_environment_sky::Sky` and calls its hooks:

- `prepare_eye(queue, eye, scene_to_clip)`;
- `encode_clouds(encoder, eye)`;
- `draw(pass, eye)` before the scene;
- `draw_clouds(pass, eye)` after it;
- `set_reveal(sphere)`.

`Environment::for_space` maps a space's identifier to one of the two variants. An identifier it doesn't know gives the Minimal grid, and the client names the missing environment.

**Android packaging is tied to this repository:**

- `scripts/android.sh` hard-codes:
  - `-p tomori-quest`;
  - the application ID `io.github.ioquatix.tomori`;
  - the output path `target/debug/apk/tomori.apk`;
  - the OpenXR loader's location `android/libs`.
- The manifest is `[package.metadata.android]` in `crates/client-quest/Cargo.toml`. `cargo-apk` reads that table only from the package it builds, so a downstream crate cannot inherit it.
- `android_main` is a `#[no_mangle]` function in the `tomori` library itself.
- The APK carries an `assets` folder, but nothing reads assets at run time. The controller models are `include_bytes!`, and the folder carries only licence notices.
- The APK is built in the `dev` profile, signed with the local debug keystore, `$HOME/.android/debug.keystore`. CI uploads it as a workflow artifact kept for 7 days. No release includes it.

## 1. Target crates

Five client-side crates replace `tomori-quest`. Each depends only on crates above it in this list. The folders follow `design/crates.md`: `crates/<role>`, and `crates/client-<target>` for clients.

| Crate | Folder | Holds | Depends on |
| --- | --- | --- | --- |
| `tomori-scene` | `crates/scene` | The platform-neutral client: the frame step and per-platform logic, the conversation and connection, the panels' layout, and audio and haptic logic behind traits. | `tomori-client`, `tomori-protocol`, `tomori-security`, `tomori-discovery`, `glam`, `font8x8`, `base-markdown` |
| `tomori-render` | `crates/render` | The wgpu renderers and shaders, controller models, the scenery contract and the shared test GPU. | `tomori-scene`, `wgpu`, `gltf`, `basisu`, `image` |
| `tomori-client-desktop` | `crates/client-desktop` | The desktop window, desktop audio, the server-less demo, the desktop participant, the console and `discover`. | `tomori-scene`, `tomori-render`, `winit`, `cpal` |
| `tomori-client-spectator` | `crates/client-spectator` | The spectator: its feed, its view, the PipeWire output and the headless mode. | `tomori-scene`, `tomori-render`, `tomori-client-desktop` |
| `tomori-client-quest` | `crates/client-quest` | The Quest shell (OpenXR, Vulkan, Android and AAudio), the builder, `android_main`, and the `tomori` executable's command line. | all of the above, `openxr`, `ash`, `android-activity`, `ndk`, `jni` |

`tomori-environment-sky` comes to depend on `tomori-render` for the scenery contract (step 5), and stops being a dependency of `tomori-render`.

### What moves where

The paths below are today's, in `crates/client-quest/src`.

**`tomori-scene`.** None of it may depend on a rendering, windowing or platform audio crate (`design/clients.md`). It builds for every client target, including `aarch64-linux-android`.

- **The frame step:**
  - `scene.rs`, `scene/headset.rs` and `scene/desktop.rs` move as they are. The per-platform logic stays in covered modules, as #111 decided.
  - `scene_graphics.rs` does not move here: it applies a `Frame` to the renderer, so it goes to `tomori-render`.
- **The conversation and connection:**
  - `conversation.rs`, `connection.rs`, `identity.rs`, `server_choice.rs` and `log_cadence.rs`.
  - `capture.rs` and `playback.rs`, without their `android` modules: the `Capture` and `Playback` traits, `Recording`, `Output`, rate clamping, downmixing and rendering into a device's format.
- **Arrival, menus and panels:**
  - `arrival.rs`, `arrival_motion.rs`, `hand_menu.rs`, `controller_ui.rs`, `panel_anchor.rs`, `pill.rs`, `recording.rs`, `transcript.rs`, `speakers.rs`, `caption.rs`, `markdown_lite.rs` and `ui_text.rs`.
  - `ui_panel.rs`'s `Canvas`, `Rect`, `Vertex` and `Canvas::mesh`, as `canvas.rs`. Its wgpu `Pipeline` and `Panel` go to `tomori-render`.
  - From `arrival_graphics.rs`: `Beam`, the ray lengths and what the frame step reads. Its pipeline goes to `tomori-render`.
  - From `workspace_graphics.rs`: `CardNavigation` and the page layout. Its vertex building and pipeline go to `tomori-render`.
- **Pointing and the swarm:** `pointing.rs`, `interaction.rs`, `swarm.rs`, `delegation.rs` and `hands.rs`.
- **Controllers:** from `controller.rs`, `Pose`, `LEFT` and `RIGHT`. The GLB and KTX2 decoding goes to `tomori-render`.
- **Feedback and audio logic:**
  - `feedback.rs`;
  - `haptics.rs`, the logic behind #130's `Vibrator` trait;
  - `spatial_audio.rs`.
- **Environments:** `environment.rs`. Step 5 replaces it with identifiers.
- **Test support:** `test_support.rs`, behind a `testing` feature, so that the other crates' tests share one TLS test server and the fake devices. It is covered by `tomori-scene`'s own tests, as `tomori-environment-sky`'s `test-gpu` feature is today.

The cycle between `conversation.rs` and `connection.rs` stays inside the one crate, so this step needs no redesign. Whether the conversation and connection belong in `tomori-scene` at all is open question 3.

**`tomori-render`.** This is the "rendering code shared by clients" crate of `design/clients.md`, and #90's §4 `tomori-render`. It grows there into the toolkit that #77 describes.

- `graphics.rs` (the `Renderer`) and `swarm.wgsl`.
- The GPU halves split out of other files:
  - `ui_panel.rs`'s `Pipeline`, `Panel` and `ui_panel.wgsl`;
  - `arrival_graphics.rs`'s pipeline and `workspace.wgsl`;
  - `workspace_graphics.rs`'s vertex building and pipeline.
- `controller_graphics.rs` and `controllers.wgsl`; `controller.rs`'s GLB and KTX2 decoding; and `assets/controllers/` with its licence. The models are compiled in.
- `scene_graphics.rs`.
- `caption.wgsl`, wherever it is used.
- `test_gpu`, moved from `tomori-environment-sky`'s `test-gpu` feature to a `test-gpu` feature here. Sky's tests then use it from here.
- In step 5, the `Scenery` contract and the registry of sceneries.

**`tomori-client-desktop`.** These are the desktop modes, which share one window and audio stack.

- `window.rs`, `desktop_window.rs` (excluded from coverage) and `desktop_audio.rs` (excluded).
- `desktop.rs`, the server-less demo; `desktop_participant.rs`; `console.rs`; and `discover.rs`.
- `desktop_window.rs` stops constructing contents itself. It runs any `window::Contents` it is given, so that the spectator crate passes its own contents without the window depending on the spectator.

**`tomori-client-spectator`.** This is the spectator client that `design/clients.md` names.

- `spectator.rs`, `spectator_feed.rs`, `spectator_view.rs`, `headless_spectator.rs`, `pipewire_video.rs` and `child_process.rs` (excluded).
- The spectator's preview window is `tomori-client-desktop`'s window, given the spectator's contents. Its speaker is `tomori-client-desktop`'s `desktop_audio`.

**`tomori-client-quest`.** This is the thin Quest shell, the builder and the app.

- `xr.rs` and `vulkan.rs` (both excluded), `xr_types.rs`, and `android.rs`.
- The `android` modules of `capture.rs` and `playback.rs`, as `android_audio.rs`. They are compiled only for Android, so the host coverage run never counts them.
- `lib.rs`:
  - the builder (section 2);
  - `android_main` and the Android event pump, behind the default `app` feature (section 3);
  - the `desktop_connection` and `android_connection` wiring.
- `main.rs` and `cli.rs`: the `tomori` executable.
  - On the host, its modes call `tomori-client-desktop` and `tomori-client-spectator`, and `--xr` calls its own OpenXR path.
  - Its command line, `--help` text and behaviour stay as they are.
- `tests/command.rs`, which runs the `tomori` executable.
- `[package.metadata.android]`. The library and executable stay `tomori`, the application ID stays `io.github.ioquatix.tomori`, and the APK stays `tomori.apk`.

The spectator modes stay in the `tomori` executable (`tomori --spectator…`), so `docs/slack-huddle.md` and scripts keep working. A separate `tomori-spectator` executable could be added later; see open question 2.

### Which are published

The recommendation is to publish all five, together with `tomori-environment-sky`, but only from step 7, once the builder has proved the interfaces with a downstream example.

- **`tomori-client-quest`** is what a downstream headset depends on. A published crate may depend only on published crates (`design/crates.md`), and its executable uses `tomori-client-desktop` and `tomori-client-spectator`. Making those host modes optional features would not avoid publishing them: an optional dependency must be published too.
- **`tomori-environment-sky`** is the default build's scenery, so the published `tomori-client-quest` needs it. Its scenery hooks become the `tomori-render` contract in step 5, which removes the reason `design/crates.md` gives for holding it back. See open question 1.
- Until step 7, every new crate is `publish = false`. That keeps their large, crate-internal interfaces from becoming a published promise early.
- All are 0.x, released at the workspace's one version. Their documentation says the interfaces change with the environment framework (#75, #77).

## 2. The headset builder

**Aim.** A downstream crate builds its own headset that registers its sceneries, and later its environments' client halves, without changing Tomori's source. The `tomori` app becomes `Builder::default()`.

Only what exists today, and what #90 has already agreed, is given an interface now:

- **Sceneries exist today.** Sky is one, drawn natively through the per-eye hooks listed above.
  - \#90 §1 says a client half provides "optional scenery, drawn natively as Sky is".
  - \#90 §4 says "sceneries and client halves use the same contract".
  - So the contract that Sky already implements becomes `tomori_render::Scenery`.
- **Client halves wait for #75.** Their trait, their state replicas (#76) and their protocol version are not designed yet. The builder gains `environment(…)` when #75 defines them, not before.

### The scenery contract, in `tomori-render`

```rust
/// What a scenery is given when the renderer creates it.
pub struct Target<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub format: wgpu::TextureFormat,
    pub width: u32,
    pub height: u32,
    pub samples: u32,
}

/// Scenery drawn natively in the eye pass, as Sky is. Renamed from Sky's own hooks, unchanged in what they do.
pub trait Scenery {
    /// Before the eye's pass: per-eye uniforms.
    fn prepare(&mut self, queue: &wgpu::Queue, eye: usize, scene_to_clip: glam::Mat4);
    /// Before the eye's pass: work in its own passes, such as Sky's clouds.
    fn encode(&mut self, _encoder: &mut wgpu::CommandEncoder, _eye: usize) {}
    /// In the eye's pass, before the scene.
    fn draw(&self, pass: &mut wgpu::RenderPass<'_>, eye: usize);
    /// In the eye's pass, after the scene and before the panels.
    fn draw_translucent(&self, _pass: &mut wgpu::RenderPass<'_>, _eye: usize) {}
    /// While a space's scenery grows out of it, only inside this sphere (centre, radius).
    fn set_reveal(&mut self, reveal: Option<(glam::Vec3, f32)>);
}

/// The sceneries a client can draw, by the identifiers spaces name them by. The Minimal grid is built in, and it is what a client shows for an identifier it doesn't know.
#[derive(Default)]
pub struct Sceneries { /* identifier → constructor */ }

impl Sceneries {
    pub fn register(&mut self, id: impl Into<String>, create: impl Fn(&Target) -> Box<dyn Scenery> + Send + Sync + 'static);
    pub fn contains(&self, id: &str) -> bool;
}
```

What changes to fit it:

- `Environment::{Minimal, Sky}` becomes an identifier, checked against `Sceneries`.
  - `Environment::for_space` becomes `Sceneries::contains`.
  - The desktop window's **E** key cycles through the registered identifiers, and `TOMORI_ENVIRONMENT` names one.
  - Behaviour is unchanged: Sky and the grid, with the grid and the missing name for an unknown identifier.

- `graphics.rs` holds `Option<Box<dyn Scenery>>` in place of its `Sky` field.

- `tomori-environment-sky` implements `Scenery` and exports a constructor:

  ```rust
  pub fn register(sceneries: &mut tomori_render::Sceneries)
  ```

- The registry later becomes the list of environments that a client sends in `Hello` (#90 §1). That needs #75's version agreement, so this plan sends nothing new.

### The builder, in `tomori-client-quest`

```rust
pub struct Builder { sceneries: Sceneries, /* … */ }

impl Default for Builder {
    /// Tomori's own build: Sky, and the grid.
    fn default() -> Self;
}

impl Builder {
    /// No sceneries but the built-in grid.
    pub fn empty() -> Self;
    pub fn scenery(self, id: impl Into<String>, create: impl Fn(&Target) -> Box<dyn Scenery> + Send + Sync + 'static) -> Self;
    pub fn sceneries(&mut self) -> &mut Sceneries;
    // Added by #75: pub fn environment(self, …) -> Self;

    /// The headset: OpenXR on Android, until the activity is destroyed.
    #[cfg(target_os = "android")]
    pub fn run_android(self, app: AndroidApp);
    /// The host's OpenXR runtime, as `tomori --xr` does.
    #[cfg(not(target_os = "android"))]
    pub fn run_xr(self, overrides: Overrides) -> anyhow::Result<()>;
}
```

- `tomori-client-desktop` gets the same registration, through a `Sceneries` passed to its runners, so that a downstream environment also shows in the desktop window and the spectator.

- The executable's command line becomes a function that takes a `Builder`:

  ```rust
  pub fn main(builder: Builder) -> anyhow::Result<()>
  ```

  A downstream desktop binary is then one line long.

- The names follow `tomori-server`'s builder where it has the same concept, once its PR lands. For example, both use `Builder` and register with a method per kind.

- `examples/headset` registers a placeholder scenery, such as a coloured floor:
  - CI builds it for the host and packages it for Android;
  - its test shows the placeholder drawn on the shared test GPU when a space names it;
  - this is #73's "an example downstream crate" for the headset half.

## 3. Reusable Android packaging

What a downstream headset needs, and how each need is met:

| Need | How |
| --- | --- |
| Its own application ID, label, APK name and icon | Its own `[package.metadata.android]`. `cargo-apk` reads that table only from the package it builds, so the downstream copies the manifest: `docs/android.md` gives it as a template, with each Tomori-specific line marked. |
| Its own extra assets | Its manifest's `assets`. Tomori reads no APK assets, because its models are compiled in, so the downstream's folder holds only its own assets. Tomori's licence notices are shown in the template, for the downstream to ship. |
| Its own `android_main` | `tomori-client-quest`'s `android_main` sits behind a default feature, `app`. A downstream crate depends on it with `default-features = false`, and writes its own entry point, shown below. Otherwise the two `#[no_mangle]` symbols would collide. |
| The OpenXR loader | `scripts/fetch-loader.sh` takes the destination folder as an argument. The template's `runtime_libs` points to that folder. |
| A build command | `scripts/android.sh` takes the package with `--package` (default `tomori-client-quest`). It reads the application ID and `apk_name` from `cargo metadata`, rather than hard-coding them. A downstream repository can use a copy of the script. |

A downstream crate's entry point:

```rust
#[unsafe(no_mangle)]
fn android_main(app: tomori::AndroidApp) {
    tomori::Builder::default().scenery(…).run_android(app)
}
```

`tomori-client-quest` re-exports `AndroidApp`, so that the downstream's `android-activity` version cannot differ from Tomori's.

Intent extras stay `tomori.server`, `tomori.name`, `tomori.token`, `tomori.msaa` and `tomori.echo_test`, since they belong to the framework, not the app. The log tag stays `tomori`. The app's own data folder already follows its application ID, so two headset builds keep separate identities.

**To check in step 6:** that `crate-type = ["cdylib", "rlib"]` on a dependency does not break or slow a downstream build. Cargo builds a dependency's `cdylib` too. If it does, the fallback is a two-line `app/` package for Tomori's own APK, which keeps the `tomori` library name and the application ID. That is open question 7.

## 4. The APK as a release artifact

**The release build.**

- `publish.yml` gains an `apk` job. It runs after `publish`, only when a release is made.
- It builds `cargo apk build -p tomori-client-quest --lib --release`.
- It checks the signing certificate with `apksigner verify --print-certs` against a fingerprint committed in the repository.
- It uploads two files to the GitHub Release that `cargo bake cargo:release:publish` creates, with `gh release upload`:
  - `tomori-<version>.apk`;
  - its `.sha256`.
- `cargo-apk` gives the APK `versionName` and `versionCode` from the workspace version.

**Signing.**

- `cargo-apk` never creates a keystore for the release profile. It reads two environment variables:
  - `CARGO_APK_RELEASE_KEYSTORE`, the keystore's path;
  - `CARGO_APK_RELEASE_KEYSTORE_PASSWORD`, its password.
- The keystore and its password are secrets of a new GitHub environment, `android-release`.
  - The keystore is base64-encoded and written to `$RUNNER_TEMP` only for that step.
  - The environment has the maintainer as required reviewer and is limited to `main`, as `crates-io` is.
  - No pull request job can read them.
- Each PR's `android` job also builds the release profile, signed with a throwaway key generated by `keytool` in the job. This catches release-only failures before a release, without the real key.
- The local debug keystore is never used for releases.
- **The key itself needs the maintainer.**
  - It identifies the app for every future update.
  - Losing it means anyone who installed the APK must uninstall it before installing a newer one. Uninstalling loses the stored identity and tokens.
  - The maintainer generates it, keeps an offline backup, and adds it to the environment.
  - The agent never sees it.
- **Moving the maintainer's headset from debug to release signing** needs one uninstall, and that loses its stored identity and tokens. Local `scripts/android.sh run` builds stay debug-signed.

## 5. Steps

Each step is one pull request that can land on its own:

- it keeps every crate at 100% region coverage once the gate is required;
- its excluded files are listed in its own crate's `Cargo.toml`, with reasons;
- it passes `check` and `test-result`;
- the `tomori` executable, the APK, the application ID and the user-visible behaviour don't change.

**0. Prerequisite.** The coverage gate, #130 and #131, lands. It is made required, so that each move is checked rather than eroded.

**1. Rename `tomori-quest` to `tomori-client-quest`.** The folder doesn't change.

- Change the package name, Cargo.lock, every `-p tomori-quest`, and the build command `cargo apk build -p tomori-client-quest --lib` in `scripts/android.sh` and CI.
- Update `readme.md`, `docs/naming.md` and `docs/slack-huddle.md`.
- Update `design/crates.md` and `design/clients.md`: each client is named `tomori-client-<target>`, so `crates.md`'s exception for clients' folders goes. The PipeWire stream names (`tomori-quest-microphone`) are run-time names and stay.
- Check the result:
  - `[lib] name` and `[[bin]] name` stay `tomori`;
  - `apk_name` stays `tomori` and the application ID stays `io.github.ioquatix.tomori`;
  - `target/debug/apk/tomori.apk` is still produced;
  - `cargo publish --workspace --dry-run --locked` still passes for the published crates.
- No coverage change.

**2. Extract `tomori-scene`.**

- Move the modules listed above, and split the four mixed files. Their GPU halves stay in `tomori-client-quest` until step 3.
- Move AAudio into `android_audio.rs`.
- Add `testing` for `test_support`.
- Add a test that fails if `cargo tree -p tomori-scene -e normal` reaches `wgpu`, `winit`, `cpal`, `ndk`, `openxr` or `ash`. This is `design/clients.md`'s validation, run in CI.
- Add an Android build of `tomori-scene` on its own beside `tomori-client`'s, as the CI comment requires for the core.
- Module paths become `tomori_scene::conversation` and so on, and the moved code becomes `pub`.
- **Coverage:** the tests move with their code. Making modules public stops `dead_code` warnings for unused items; the coverage gate now finds them instead.

**3. Extract `tomori-render`.**

- Move the renderers, the shaders, the GPU halves, the controller models and `scene_graphics.rs`.
- Move `test_gpu` from Sky to here.
- **Coverage:** the GPU tests run on lavapipe, as today.

**4. Extract `tomori-client-desktop`, then `tomori-client-spectator`.** These are two PRs.

- `desktop_window.rs` takes its contents from the caller.
- The excluded files move with their exclusions.
- The `tomori` executable dispatches to the two crates; `tests/command.rs` is unchanged.
- This closes #56.

**5. Sceneries and the builder.**

- Add `tomori_render::Scenery` and `Sceneries`.
- Sky implements `Scenery`.
- The identifiers replace the `Environment` enum.
- Add `Builder`, with `run_android` and `run_xr`.
- The runners in `tomori-client-desktop` take `Sceneries`.
- Add `examples/headset`, built in CI for the host and Android.
- **Coverage:** the builder's wiring is tested on the host through `run_xr`'s setup, up to the runtime that CI does not have. `android_main` is Android-only and not compiled on the host.

**6. Reusable Android packaging.**

- Add the `app` feature, the `--package` argument of `scripts/android.sh`, the loader's destination argument and `docs/android.md`'s template.
- `examples/headset` is packaged in CI with its own application ID, which proves the template.

**7. Publish.**

- Remove `publish = false` from the five crates and Sky.
- Add their metadata, readmes and `license.md`.
- Update `design/crates.md`'s tables.
- Check that `cargo publish --workspace --dry-run --locked` and `cargo bake cargo:packages` list them.

**8. The APK release artifact.**

- Add the `apk` job in `publish.yml`, the release-profile build with a throwaway key in `test.yml`, and the `android-release` environment.
- The release documentation in `readme.md` and `releases.md` says where the APK is published and how to verify its certificate.
- The maintainer adds the key before the first release that includes the job.

**Ordering.**

- Steps 1–4 are moves, with only the splits that a crate boundary forces.
- Steps 5–6 add the new interfaces.
- Steps 7–8 release.
- Steps 2–4 each touch many files in `crates/client-quest`. They should land one at a time, between other client work, rather than run beside it. Each rebase is mostly a matter of moved paths.

## Risks

- **Merge conflicts.** Steps 2–4 move most of the client. Any client PR open during a move has to be rebased onto the new paths.
  - Mitigation: land each move quickly, announce it, and keep moves free of edits other than the forced splits.
- **A large published interface.** Most of the client's logic, over 20,000 lines, becomes `pub` in `tomori-scene`.
  - Mitigation: they stay unpublished until step 7, and remain 0.x after that.
  - The conversation's internals could be narrowed later with `pub(crate)` where other crates don't use them.
- **Coverage across crates.** A workspace-wide coverage run counts a crate's code when another crate's tests exercise it. A crate tested alone, or after publication, may then be less covered than the gate shows.
  - Mitigation: each crate's own tests cover it. `tests/command.rs` in `tomori-client-quest` checks only the command line.
- **Compile times.** More crates build more in parallel, but the `cdylib` and `rlib` pair may build twice for a downstream crate (step 6).
- **Packaging stays on `cargo-apk` 0.10**, pinned in CI. The template in step 6 ties downstream builds to its manifest format. Replacing it would be a separate decision.
- **Signing.** Losing the release key strands installed copies, and a leak lets others sign updates. Hence the separate environment, a required reviewer, an offline backup and a committed fingerprint.
- **Divergence between the desktop and Quest.** #111 decided that their UIs may diverge. Putting per-platform logic in `tomori-scene` keeps it covered, but it must not force one UX. `scene/headset.rs` and `scene/desktop.rs` remain separate.

## Decisions

The maintainer answered the open questions on #132:

1. **What to publish:** all five crates: `tomori-scene`, `tomori-render`, `tomori-client-desktop`, `tomori-client-spectator` and `tomori-client-quest`. `tomori-environment-sky` stays private until the shared environment interface, #75, exists. This conflicts with the dependencies as they stand; see [Progress](#progress).
2. **The spectator:** `tomori-client-spectator` has its own executable, `tomori-spectator`. The OBS and huddle docs use it.
3. **Placement:** the conversation state lives in `tomori-scene`, and the connection in the existing `tomori-client` crate.
4. **The release APK:** kept. It is signed with a private release key, stored as a secret in a protected GitHub environment, such as `android-release`, deployable only from `main` with the maintainer's approval. The maintainer keeps the key's backup. Pull requests build the release profile with a throwaway key, as section 4 proposes.
5. **The application ID** moves to `com.codeotaku.tomori`, once. The new app installs alongside `io.github.ioquatix.tomori`, and the headset is paired again; the old app is removed when ready. `docs/naming.md` changes with it. This is in a later phase, with packaging.

## Progress

The first phase is steps 1–5, as one pull request each:

| Step | Pull request | State |
| --- | --- | --- |
| Housekeeping: `releases.md`'s duplicated entry | #138 | Merged |
| 1. Rename to `tomori-client-quest` | #139 | Merged |
| 2. Extract `tomori-scene` | #144 | Draft, for review |
| 3. Extract `tomori-render` | #150 | Draft, stacked on #144 |
| 4. Extract `tomori-client-desktop` and `tomori-client-spectator` | #160 | Draft, stacked on #150 |
| 5. Sceneries and the builder | #161 | Draft, stacked on #160 |

Steps 6–8 (packaging, the application ID, publishing and the release APK) are the next phase.

What the implementation changed from this plan:

- **`tomori-scene`:**
  - `capture.rs` and `playback.rs` also held code in cpal's terms (`Mono`, `input_failed`, `Render`, `output_failed`). It became `cpal_audio.rs`, which moved with `desktop_audio.rs` to `tomori-client-desktop`, so the scene doesn't depend on cpal.
  - `workspace_graphics.rs`'s navigation and paging became the scene's `cards.rs`, and `ui_panel.rs`'s `Canvas` its `canvas.rs`. `controller.rs`'s `Pose`, `LEFT` and `RIGHT` became the scene's `controller.rs`, and `Beam` and the ray lengths moved to `pointing.rs`.
  - Test-only helpers that other crates' tests use, such as `scene::fixtures` and `system::refuse`, are behind the `testing` feature too.
  - `tests/dependencies.rs` runs `cargo tree` for the host and Android, and fails if the scene reaches a rendering, windowing or device crate.
- **`tomori-render`:**
  - `test_gpu` stays in `tomori-environment-sky` for now. Sky's own tests use it, and the render crate depends on Sky, so moving it would need a dev-dependency cycle. It moves once Sky depends on `tomori-render`.
  - The controller models and their licence notices are in `crates/render/assets`. The APK's manifest points `assets` there, so the APK carries the same files.
- **Desktop and spectator:**
  - These are one pull request, because the spectator's window was the same contents type as the server-less window, `Demo`. `Demo` is now the server-less window only, and the spectator crate has its own `SpectatorWindow`.
  - `desktop_window.rs` keeps the server-less and participant runners and gains `show(settings, open, with)` for other crates' contents. The closures that start contents once the window has a device stay in that excluded file.
  - `tomori --spectator…` still works beside `tomori-spectator`.
- **The connection moves to `tomori-client` in #174**, after the builder, as its own design change:
  - The boundary is a trait, `tomori_client::connection::Session`, which the scene's `Conversation` implements, so the scene depends on the client and never the reverse. A message channel was not needed: the connection already runs on the frame's thread.
  - The connection needs five things from a session: `next_action`, taken one at a time, so that a report can withdraw what is queued behind a failure; `apply` for the client's events; `report` for its own `Report`s (the server given at startup, the kept Feedback setting, focus loss, the servers found, the frame's tick, the connection generation, failures, the microphone failing, words not sent, security notices and replaying); `is_browsing`; and `observe`, which tells the feedback model what waits for the participant.
  - The scene needs from the connection only what it had: `poll`, `feedback`, `request_microphone`, `send_presence` with protocol poses, and `listen`, which hands the scene the other participants' heads so that it places their voices on its seats.
  - The identity and its store of certificates and tokens, the server choices, the feedback model, and the `Capture` and `Playback` traits moved with it; the helpers for device adapters stay in the scene.
  - The tests drive the connection through a scripted session, against a scripted peer and the real test server. `tomori-client` dev-depends on `tomori-server` by path only, as Sky does on `tomori-render`, so that packaging leaves the cycle out.
- **Sceneries and the builder:**
  - `Scenery::prepare` and `encode` take `&self`, as Sky's hooks do, because the renderer draws with `&self`.
  - `Scenery` gains `resize`, which Sky needs.
  - Sky implements `Scenery` through an adapter in `tomori-render`, so the dependency is not inverted yet.
  - `Environment` is now a `Copy` identifier, with `MINIMAL` and `SKY` constants. `Conversation::set_drawable` gives a conversation the environments its build draws.
  - The builder registers sceneries by `&'static str` identifiers, and runs a build with `run_android`, `run_xr`, `run_desktop`, `run_demo` or `tomori::main(builder)`.
  - The spectator and the console still draw Tomori's own sceneries.
  - The example is `examples/headset`. It draws a placeholder floor for `examples/server`'s placeholder environment, and is tested on the shared test GPU. CI does not package it for Android yet; that is step 6.
- **Publishing conflicts with Sky staying private.** `tomori-render` depends on `tomori-environment-sky` for Tomori's default scenery, and every client crate depends on `tomori-render`. So none of them can be published while Sky is not: `cargo package` stops at `tomori-render`. `tomori-scene` packages and verifies. Before step 7, either Sky is published, or Tomori's own Sky registration moves out of the published crates, for example into an unpublished `tomori` app package, which open question 7 already considered.

## Open questions for the maintainer (answered above)

1. **What to publish.** Should all five crates and `tomori-environment-sky` be published at step 7? Or should the APK be the only release of the headset at first, with the client crates kept unpublished until a downstream crate needs them?
2. **The spectator's executable.** Keep `tomori --spectator…` as it is (the recommendation), or add a `tomori-spectator` executable to `tomori-client-spectator`? Moving it would change `docs/slack-huddle.md` and any script that runs the spectator.
3. **The conversation and the connection.** Should these live in `tomori-scene`, which this plan recommends because the frame step depends on them and they depend on each other? The alternative is a separate client-session crate, such as a layer of `tomori-client`. The name `tomori-scene` also covers #77's generic scene, which may make it too broad.
4. **The release key.** Who generates it, where its backup is kept, and is the `android-release` environment, with you as required reviewer, acceptable?
5. **The release artifact's name.** `tomori-<version>.apk`, or `tomori.apk` as built?
6. **Your headset.** Is it fine to move it to release signing at the first release, accepting one uninstall and new pairing? Or should it stay on debug builds?
7. **Packaging.** Is the `app` default feature acceptable, or should Tomori's own APK come from a separate small package from the start?
