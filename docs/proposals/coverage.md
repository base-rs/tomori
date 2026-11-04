# Proposal: Reaching 100% Region Coverage When Some Code Can't Run in CI

Status: **for discussion**, November 2026. Nothing here is decided or implemented. It covers [#55](https://github.com/ioquatix/tomori/issues/55) and touches stage 3 of [#73](https://github.com/ioquatix/tomori/issues/73), the server and headset builders.

## The request

From a conversation in the headset, November 2026:

- **Why 100%.** The target is 100%, not a floor such as 80%.
  - The last 10% finds as many bugs as the first 90%, and it exposes dead code.
  - With a floor, people end up writing tests for code they don't care about when someone else's change lowers the total.
  - Each change should sort out its own gaps and add its tests when it's made.
- **The problem.** Some code can't realistically run in GitHub CI, such as the Quest app's OpenXR and Android parts.
- **The idea.** Metadata in a sub-crate's `Cargo.toml` tells Bake's coverage task to skip that crate, and says why. A project can then state plainly that one crate isn't realistically testable in GitHub CI, much as third-party dependencies don't count.
- **Agreed in principle.**
  - An explicit crate-level exclusion with a required reason, listed in the summary, is better than a lower target or ignore markers scattered through the code.
  - The excluded crate must stay thin. Logic such as gesture thresholds, panel placement, pose maths, input sampling and haptic mapping moves into covered crates.
- **Not decided.** "Let's not make any changes yet; analyse the problem and think about the solution space."

## Recommendation in brief

- **One exclusion mechanism: a whole crate.** Add `[package.metadata.bake.test.coverage]` to `bake-test`, with `exclude = true` and a required, non-empty `reason`.
  - The excluded crate's tests still run and must pass. Only its regions stop counting.
  - The summary always names each excluded crate, its reason, its size in regions and how much of it the tests reach anyway. The exclusion can't quietly grow.
  - No file regexes, no `#[coverage(off)]`, no `cfg(coverage)` stubs.
- **Measure only the workspace.** `bake-test` should count only regions under the workspace root. Today it fails on hosts whose standard library sources live outside rustup, such as Arch's `/usr/src/debug/rust`.
- **Make the excluded crate thin by splitting `tomori-quest`.**
  - A platform shell keeps the `tomori-quest` name, the APK, the binary and every OpenXR, Vulkan-interop, Android, winit, cpal and PipeWire call. It is excluded.
  - Everything else moves to covered crates: the app logic, with the #73 builder, and the renderers, tested on lavapipe.
  - The key change is one per-frame step function that takes plain input and returns plain output. The OpenXR loop and the desktop window both feed it. Today that logic is written twice, inside `xr::run` and `desktop_participant::State::frame`, and neither copy is tested.
- **Keep it thin with two checks:**
  - a review rule: the shell makes no decisions;
  - an optional size limit in the same metadata, so growing the shell needs a visible change to `Cargo.toml`.
- **Examples stay unmeasured,** as cargo-llvm-cov already arranges. Anything an example needs beyond argument parsing and printing belongs in a library, where it's covered.
- **Device and self-hosted runs** can provide evidence, but they don't count toward the gate.

## What the numbers say

From a coverage run of `origin/main` on 2026-11-04 (`cargo bake --locked test:coverage --all-targets true`, all tests passing):

| | Regions | Uncovered | Covered | Share of the gap |
| --- | ---: | ---: | ---: | ---: |
| Workspace | 92,035 | 15,782 | 82.9% | 100% |
| `tomori-quest` (`crates/client-quest`) | 47,006 | 10,548 | 77.6% | 66.8% |
| `tomori-server` | 18,791 | 1,713 | 90.9% | 10.9% |
| `tomori-backend-opencode` | 12,648 | 1,292 | 89.8% | 8.2% |
| `tomori-speech` | 5,206 | 1,234 | 76.3% | 7.8% |
| `tomori-environment-sky` | 1,401 | 568 | 59.5% | 3.6% |
| Others | 6,983 | 427 | 93.9% | 2.7% |

CI's figure is 76,322 of 92,022 (82.94%). The small difference is the `unreachable!()` exclusion.

The analysis also sorted uncovered functions into categories. This view counts about 17,800 regions, because it includes roughly 2,000 in `examples/` and `tests.rs` files that the gate doesn't measure (see [What is configurable](#what-is-configurable)):

| Category | Share | Where | Can CI run it? |
| --- | ---: | --- | --- |
| OpenXR and platform glue | 20% | `xr.rs`, `desktop*.rs`, `vulkan.rs`, `pipewire_video.rs` | Partly. Most of `xr::run` is interaction logic, not OpenXR calls. |
| Bodies of ignored GPU tests | 19% | inline test modules in `graphics.rs` and `workspace_graphics.rs` | Yes, on lavapipe. In progress. |
| Networking and error paths | 18% | both `connection.rs` files, `hub.rs`, `wire.rs`, `api.rs` | Yes. Server side in progress. |
| GPU and rendering | 12% | renderers, `environment-sky` | Yes, on lavapipe. |
| Examples | 7% | `recognition_benchmark.rs`, `presence_sender.rs`, `connect.rs` | Not measured today. |
| CLI and `main` | 7% | `tomori-server`'s `main.rs`, `console.rs`, `discover.rs` | Yes, with injected arguments and I/O. |
| Untested pure logic | 5% | `conversation.rs`, `transcript.rs`, `markdown_lite.rs` | Yes. |
| Speech models and their ignored tests | 7% | `kokoro.rs`, `transducer.rs`, `vad.rs`, `offline.rs` | Yes, if CI fetches and caches the models. |
| Test helpers' unused branches | 3% | various | Yes, or remove the branches. |
| Audio devices, multicast tests | 2% | `capture.rs`, `playback.rs`, `discovery` | Partly. |

**What lavapipe does.** Run on lavapipe, 10 of the 11 ignored GPU tests pass and close 4,135 regions, which takes the workspace to about 87.4%. The renderer files go from 56.7% to 91.4%. One test, `vulkan_sign_images_include_post_text_and_smooth_fade_in_both_eyes`, fails on llvmpipe only.

**Two things the totals hide:**

- **Android-only code is never measured.** About 675 lines under `cfg(target_os = "android")` aren't even compiled by a host coverage run:
  - `android.rs`;
  - the Android parts of `lib.rs`: `android_main`, `AndroidEventPump` and the Android connection;
  - the AAudio modules of `capture.rs` and `playback.rs`.

  Nothing reports them, so they're the silent erosion this proposal wants to prevent.
- **Some code exists only for Android.** `capture.rs`'s `run_capture`, `drain` and `InputStream`, and `graphics::requested_samples`, are compiled on the host but called only from Android code, so the host build flags them as dead.

**What is truly beyond GitHub CI** comes to about 3,600 regions:

- the OpenXR calls in `xr::run`;
- `vulkan.rs`, the Vulkan device that OpenXR requires;
- the winit event handlers;
- PipeWire output.

That's about 4% of the workspace. An exclusion sized to that, and no bigger, is the goal.

## The solution space

Each option is judged against the goals: 100% of everything that counts, exceptions that are explicit and reviewable, and no silent erosion.

### 1. Crate-level exclusion in Bake metadata

The excluded crate is declared in its own `Cargo.toml`, with a reason; the gate skips its regions and the summary names it.

- **For:**
  - It's one switch per crate, in the crate's own manifest, so a review sees it.
  - It matches how dependencies already don't count.
  - It forces the useful restructuring: you can't exclude a crate full of logic without excluding the logic, which pushes logic out of it.
  - A crate is a real boundary. The compiler enforces which way the dependencies go, so logic can't leak back into the shell without a visible dependency change.
  - It covers Android cleanly: if every `cfg(target_os = "android")` item lives in the excluded crate, nothing unmeasured remains elsewhere.
- **Against:**
  - It's coarse: it needs a crate split before it helps.
  - An excluded crate could grow unnoticed if the summary only says "excluded". So the summary must report its size, and a limit can be set; see below.
- **Verdict:** recommended. It is the only exclusion mechanism.

### 2. File or module exclusion via `--ignore-filename-regex` from config

A workspace setting, such as `ignore = ["crates/client-quest/src/xr\\.rs"]`, is passed to cargo-llvm-cov.

- **For:** fine-grained, and needs no restructuring.
- **Against:**
  - It invites excluding mixed files whole: `xr.rs` is mostly interaction logic, and excluding it would hide about 1,000 regions of decisions.
  - Regexes also match files added later.
  - They go stale silently when files move.
  - A module boundary is weaker than a crate's: nothing stops covered code calling into an excluded file, or excluded code growing logic.
- **Verdict:** not recommended. Reconsider it only if a crate split turns out impossible somewhere. If so, use exact paths, not regexes, each with a reason, reported like crates.

### 3. `#[coverage(off)]`

- **Status:**
  - Still unstable. Rust 1.99.1 (October 2026) rejects it with E0658 without `#![feature(coverage_attribute)]`; checked locally.
  - A stabilization was merged in December 2024 by mistake and reverted.
  - The new tracking issue, [rust-lang/rust#134749](https://github.com/rust-lang/rust/issues/134749), is open, with sign-off and documentation steps outstanding.
  - cargo-llvm-cov recommends `#[cfg_attr(coverage_nightly, coverage(off))]`, which works only when coverage is built with nightly.
- **Against:**
  - Tomori builds with stable and with its minimum version, 1.90, so this would need a separate nightly coverage job.
  - The markers would be scattered, and a summary can't list them without scanning source.
  - The compiler documents the attribute as a hint whose precise effect may change.
- **Verdict:** rejected, even once it's stable, for the scattering alone.

### 4. `cfg(coverage)`

cargo-llvm-cov sets `cfg(coverage)` on stable, so `#[cfg(not(coverage))]` can compile platform code out of the coverage build.

- **Against:**
  - The coverage build is then no longer the shipped build.
  - Tests that reach the code stop compiling.
  - Like the attribute, the exclusions are invisible in the summary.
- **Verdict:** rejected for exclusion.

### 5. Tests outside GitHub CI, with profiles merged

A self-hosted runner with a GPU or a headset, or a device lab, runs some tests, and its `.profraw` files are merged with CI's.

- **Mechanics:**
  - cargo-llvm-cov supports merging, with `show-env` and `--no-report` runs followed by `cargo llvm-cov report`.
  - A profile only maps onto the exact instrumented binary that produced it, so the device must run artifacts built in the same pipeline, and they must be shipped there and back.
- **Against:**
  - GitHub advises against self-hosted runners for public repositories, because pull requests from forks run code on them.
  - A required check that depends on one desktop or one headset fails whenever it's asleep, busy or not worn. The Quest's proximity sensor stops the session when it isn't worn.
  - lavapipe already removes the need for a real GPU.
- **Verdict:** useful as optional, non-gating evidence, such as a "device smoke" run before a release. Not part of the 100% gate.

### 6. Android coverage on the device

Build the `aarch64-linux-android` library with `-C instrument-coverage`, set `LLVM_PROFILE_FILE` to the app's files directory, run on the headset, `adb pull` the profiles and merge them against the same `.so`.

- **For:** feasible as a manual audit.
- **Against:**
  - Someone must wear the headset.
  - It needs the profiler runtime for the Android target. Whether rustup's `aarch64-linux-android` standard library ships it isn't verified here.
  - It can't run per pull request.
  - An x86_64 Android emulator on GitHub runners could run the non-XR Android code, such as AAudio and the JNI multicast lock. But the app is aarch64-only, and the code involved is small.
- **Verdict:** optional and occasional. Keeping Android code inside the excluded shell, and thin, matters more.

### 7. Testing more, with fakes for OpenXR

This comes in two forms.

- **(a) Restructure so the logic needs no OpenXR.**
  - Most of `xr::run` (lines 30–912) decides what buttons, rays, palms and holds mean: hold-to-leave, which wrist wins, when to tick haptics, where panels stand.
  - `desktop_participant::State::frame` (lines 517–885) repeats much of it for the keyboard and mouse. Its comments often say "as on the headset" or "see `xr::run`".
  - One shared frame step with plain input removes the duplication and makes all of it testable. The Base testing skill warns against "production abstractions solely to satisfy coverage"; this one also removes a real duplication.
- **(b) A real OpenXR runtime in CI.**
  - Monado, the open-source runtime, has a null compositor and simulated devices.
  - If it runs headless on a GitHub runner with lavapipe, the real `run_xr` path, which already exists for host runtimes, could run as a test, and the shell could shrink further.
  - Whether it does needs a spike.
- **Verdict:** do (a) as part of the split. Try (b) later; if it works, it shrinks the exclusion, and the metadata stays the same.

### How examples should count

- **Today.** cargo-llvm-cov's default ignore pattern already leaves out `examples/`, `tests/`, `benches/`, `tests.rs` and `*_tests.rs`, so the gate doesn't measure the examples' 1,193 regions. `--all-targets true` only builds them and runs any `#[test]` functions in them.
- **Option: run them as tests.** Run each against a fake server and measure them with `--no-default-ignore-filename-regex`.
  - That also starts measuring every `tests.rs` file, so test code would have to reach 100% too.
  - `recognition_benchmark.rs` needs speech models and real recordings.
- **Recommendation:** keep examples unmeasured but thin, like the shell.
  - An example parses arguments, calls the library and prints.
  - Anything more, such as `connect.rs`'s choosing or `presence_sender.rs`'s participant, moves into the library, where it's covered.
  - `recognition_benchmark.rs` is a tool rather than an example; it could become a `bench` or a `bin` of the speech crate. That's a question below.
- **Make it visible.** `bake-test`'s summary should say that examples, tests and benches aren't measured, so this exclusion is visible too.

The same default affects tests themselves:

- An inline `#[cfg(test)] mod tests` is measured, so ignored test bodies and unused helper branches count.
- Tests in a `tests.rs` file, as `bake-test` and `environment-sky` lay them out, aren't measured.
- Moving Tomori's inline test modules into `tests.rs` files would make the layout consistent. Their 19% "ignored GPU test bodies" mostly disappears anyway once lavapipe runs them.

## How Bake's coverage task works today

This section describes `bake-test` 0.3.4, as locked in Tomori's `Cargo.lock`, from its source in `~/.cargo/registry/src/index.crates.io-*/bake-test-0.3.4`.

### What it runs

`test:coverage`, in `src/lib.rs`, takes these options:

- `--all-features`, or repeated `--features`;
- `--package`, to limit the run to one package;
- `--all-targets`.

It then:

1. calls the optional `test:before` hook, which in Tomori fetches the sherpa-onnx libraries;
2. runs `cargo test --doc` normally, because doctest coverage is unstable;
3. runs `cargo llvm-cov --workspace --locked [--all-targets] [features] --json --output-path <temporary>/coverage.json`, or `--package <name>` instead of `--workspace`;
4. summarizes the JSON in `src/coverage.rs`:
   - It reads every file in the export.
   - It takes each function's code and expansion regions (kinds 0 and 1) that lie in those files.
   - It merges regions with identical spans across generic instantiations, so a region counts as covered if any instantiation ran it.
   - It removes regions inside `unreachable!(…)` calls; the source scan skips strings and comments.
   - It counts the rest.
5. fails if any measured region is uncovered, listing every uncovered region as `path:line:column-line:column`.

### What is configurable

- **Selection of what runs:** the package, the features and the targets.
- **The `test:before` hook.**
- **The automatic `unreachable!()` exclusion,** which has no switch.
- **cargo-llvm-cov's own defaults,** which `bake-test` doesn't override:
  - `examples/`, `tests/` and `benches/` directories, and `tests.rs` and `*_tests.rs` files, aren't reported;
  - nor are the target directory, `$CARGO_HOME/registry` and `git`, rustup's toolchains, and `rustc/<hash>/` paths.
- **Nothing else.** There's no threshold, ignore list or exclusion.
- **Metadata.** `bake-test` already reads Cargo metadata, but only for `bake.test.external`: workspace metadata, or the root package's. That sets the namespace convention for the proposal below.

### Problems found along the way

- **Sources outside the workspace.** With a distribution's Rust, such as Arch's, standard-library sources are reported under `/usr/src/debug/rust/…`, which matches none of the default patterns. The summarizer then tries to read them and fails. The analysis had to work around this with `cargo llvm-cov report --ignore-filename-regex '^/usr/src/'`.
- **Long failure output.** Tomori's `coverage.yml` comment says the failure arrives in the Actions log as one line holding every uncovered region, often megabytes long. The workflow greps the counts back out into the job summary.

### The proposed metadata

The metadata goes in the excluded crate's own manifest, under the existing `bake.test` namespace:

```toml
# crates/client-quest/Cargo.toml
[package.metadata.bake.test.coverage]
exclude = true
reason = "OpenXR session and frame loop, the Vulkan device OpenXR requires, and the Android activity, AAudio and JNI glue: they need a headset runtime that GitHub CI lacks. Logic lives in tomori-app and tomori-render, which are covered."
# Optional: fail if the crate grows past this many measured regions.
limit = 3000
```

**Schema.** These rules are enforced by `bake-test`, and an error names the manifest:

| Key | Type | Rule |
| --- | --- | --- |
| `exclude` | boolean | Required if the table is present. `false` is allowed, so an exclusion can be switched off without deleting its history. |
| `reason` | string | Required and non-empty, after trimming, when `exclude = true`. It should say what can't run and where, and where the logic went. |
| `limit` | positive integer | Optional. If the crate's measured regions exceed it, the gate fails, naming the crate, its size and the limit. |
| anything else | | Rejected, so a typo such as `reasons` fails rather than being ignored. |

**Where the table may appear:**

- Only in a workspace member's `[package.metadata]`.
- `[workspace.metadata.bake.test.coverage]` is reserved for workspace-wide settings and has no `exclude`.
- A suggested rule, not decided: Base allows `exclude = true` only in a `publish = false` crate. A published library should never ship unmeasured code, and an application shell is never published.

**Behaviour:**

- **Package list.** `bake-test` reads `cargo metadata --no-deps` for all workspace members, including unpublished ones. The existing helper skips `publish = false` crates for external tests, so it needs a variant.
- **Mapping regions to packages.** Each region is assigned to the package whose manifest directory is its path's longest prefix.
  - Regions outside the workspace root aren't counted at all. That fixes the `/usr/src` failure, and it's the principle behind the request: code outside the project doesn't count.
- **Tests still run.** The excluded crate's tests run and must pass; only its regions leave the gate.
  - `bake-test` does this classification itself, rather than passing `--exclude-from-report` to cargo-llvm-cov, so that it can still count the excluded crate's regions.
  - Code in a covered crate that is run only by the excluded crate's tests still counts as covered. That's real execution.
- **`--package` naming an excluded crate.** The run reports the crate's figures and says that nothing was gated.
- **A fully covered excluded crate.** If an excluded crate reaches 100%, the summary suggests removing its exclusion.

### The summary

The summary keeps exclusions in view whether the gate passes or fails. The figures below are illustrative:

```text
Region coverage passed: 74,630/74,630 measured source regions covered.
  Excluded by unreachable!(): 3 regions.
  Excluded crate tomori-quest (crates/client-quest): 2,480 regions, 1,104 reached by tests (44.5%), limit 3,000.
    Reason: OpenXR session and frame loop, the Vulkan device OpenXR requires, and the Android activity, AAudio and JNI glue: …
  Not measured (cargo-llvm-cov defaults): examples/, tests/, benches/, tests.rs files.
```

When the gate fails, the same lines follow the count of uncovered regions, then the region list, one per line. `bake-test` could also append the first few lines to `$GITHUB_STEP_SUMMARY` when that is set, so Tomori's grep step can go. That part is a separate change.

### Base guidance

The `base-project-testing` skill and `bake-test`'s `context/testing.md` would gain a short section:

- **When it's allowed.** A crate may be excluded only when its code calls platform interfaces that CI can't provide: XR runtimes, device drivers, mobile operating system services, windowing event loops.
- **Never for:**
  - code that is merely hard to test: injected I/O, fakes and `#[cfg(test)]` shims come first, as the skill already says;
  - logic;
  - a published crate.
- **The excluded crate is a shell.** It translates between platform types and plain types, and makes no decisions. Any branch that isn't about a platform call's result belongs in a covered crate.
- **Platform-specific code.** Code compiled only for a platform the gate doesn't run on, such as `cfg(target_os = "android")`, lives in the excluded crate, or is tested on that platform. This replaces the skill's "run coverage on each supported target" for targets CI can't run tests on.
- **Reviewing a change to an exclusion.** It needs the same scrutiny as lowering a target would. Reviewers check the reason, the size and whether logic moved in.

## Keeping the excluded crate thin: splitting `tomori-quest`

### The shape

```text
tomori-quest  (crates/client-quest; excluded; the APK, the `tomori` binary, the cdylib)
  ├── tomori-app     (crates/app; covered; the headset app logic, its builder for #73)
  ├── tomori-render  (crates/render; covered on lavapipe; wgpu renderers)
  └── openxr, ash, android-activity, ndk, jni, winit, cpal
tomori-render ──► tomori-app's plain types, tomori-environment-sky, wgpu
tomori-app    ──► tomori-client, tomori-protocol, tomori-security, tomori-discovery, glam
```

**Names.** `tomori-app` is a working name; see the open questions. `tomori-render` is the name `docs/naming.md` already plans for #97, and it can start here as the headset's renderers.

**Why this shape:**

- **It's what `design/clients.md` already requires:**
  - a platform-neutral core and scene model "free of rendering code";
  - rendering "in its own crate, not in one client that others depend on";
  - engagement derived from platform-neutral signals.

  The split brings the code in line with that design.
- **It's what #73 needs.** A downstream headset build registers its environments through a builder in `tomori-app` and uses `tomori-quest`'s shell to run, without forking either.
- **It keeps names stable.** `tomori-quest` keeps its name and its `[package.metadata.android]`, so `scripts/android.sh`, the APK name and the application ID don't change.

### The seam: one frame step

- **The new type.** `tomori-app` gains a `Frame` (or `Participant`) type with one step. It takes a `FrameInput`:
  - session focus and whether the headset is worn;
  - the display time;
  - the head and per-eye views as `glam` types;
  - `Controls`, the existing button and axis struct;
  - controller poses and hand joints, each optional.

  It returns a `FrameOutput`:
  - what the renderer should draw;
  - haptic pulses;
  - the listener's pose for spatial audio;
  - presence to send;
  - log events.
- **The shell's part.** It fills the input from OpenXR, or from the keyboard and mouse on the desktop, and carries out the output. That is all it does.
- **The tests.** They drive the step with scripted input. For example: "hold B in a space for 1 s and the participant leaves", "the left palm facing the head opens the hand menu", "a focus loss during X cancels the utterance".

### What moves where

This is a high-level, file-by-file plan. "Split" means part of the file stays as glue and the rest moves.

| File | Today | Goes to |
| --- | --- | --- |
| `main.rs`, `android.rs` | entry points, Android activity | **shell** |
| `lib.rs` | module list, `run_*` entry points, `android_main`, `AndroidEventPump` | **shell**: the entry points, which call into `tomori-app` |
| `xr.rs` | `run`, the session loop and frame body; `Input`, actions and bindings; `HandTracking`; `ControllerModels`; `SceneAnchor` and view maths | **split**: the session, swapchain, action and space calls stay in the **shell**. Into **app**: the frame body's decisions, `Controls::from_actions`, `LiveGrip`, `SceneAnchor`'s maths, `initial_scene_transform`, `head_pose` and `view_projection` (with `glam` input), and `arrival_target` |
| `vulkan.rs` | the Vulkan instance and device for OpenXR, image import | **shell** |
| `haptics.rs` | OpenXR haptic action | **split**: the action stays in the **shell**; which pulse for which event goes to **app**, next to `feedback.rs` |
| `desktop.rs` | winit app, window GPU, `Camera` | **split**: the `ApplicationHandler`, surface and acquisition stay in the **shell**; `Camera` and its ray maths go to **app**, with their tests |
| `desktop_participant.rs` | winit app, key mapping, `Head`, `State::frame` | **split**: the event handler stays in the **shell**; `Participant::key`, `Input`, `Head`, and the frame body, rewritten to use the shared step, go to **app** |
| `headless_spectator.rs` | PipeWire loop | **split**: `next_frame_time` and the draw schedule go to **app**; the loop stays in the **shell** |
| `pipewire_video.rs` | GPU readback, gstreamer child | **split**: `pack_rows` and the frame layout go to **render**; `Producer` stays in the **shell** (it is already partly tested through `from_child`) |
| `capture.rs` | `Capture` trait, downmix, cpal and AAudio modules, `run_capture`, `drain`, `InputStream` | **split**: the trait, `downmix`, `run_capture`, `drain` and `InputStream` go to **app**, tested on the host with a fake stream, which also ends their dead-code warnings; the cpal and AAudio modules stay in the **shell** |
| `playback.rs` | `Playback` trait, channel rendering, cpal and AAudio modules | **split** the same way |
| `graphics.rs`, `workspace_graphics.rs`, `arrival_graphics.rs`, `controller_graphics.rs`, `controller_ui.rs`, `ui_panel.rs`, `spectator_view.rs` and the `.wgsl` files | wgpu renderers | **render**, covered on lavapipe |
| `ui_text.rs`, `caption.rs`, `swarm.rs` | text layout, captions, the swarm simulation | **app** if free of wgpu, otherwise **render** |
| `conversation.rs`, `connection.rs`, `delegation.rs`, `transcript.rs`, `markdown_lite.rs`, `hand_menu.rs`, `arrival.rs`, `arrival_motion.rs`, `controller.rs`, `interaction.rs`, `pointing.rs`, `panel_anchor.rs`, `pill.rs`, `recording.rs`, `feedback.rs`, `spatial_audio.rs`, `speakers.rs`, `server_choice.rs`, `identity.rs`, `log_cadence.rs`, `environment.rs`, `hands.rs` | app logic and state | **app**. Most are already above 95%. `connection.rs`, with 658 uncovered regions, and `conversation.rs`, with 340, are the main gaps to test. |
| `spectator.rs`, `spectator_feed.rs` | the spectator's options and feed | **app** for now; `crates/client-spectator` later, per `design/clients.md` and #56 |
| `console.rs`, `discover.rs` | terminal panel, server listing | **app**, with their input and output injected so that `run` is tested; they need no platform |

**Estimated shell size.** Roughly 2,000–3,000 measured regions:

- the OpenXR calls left in `xr.rs`;
- `vulkan.rs` (244);
- the winit handlers;
- device opening in `capture.rs` and `playback.rs`;
- the gstreamer `Producer`;
- the entry points;
- the Android code.

That is within the ~3,600 regions the analysis found truly beyond CI, and about 3% of the workspace.

**Order of the split.** One pull request each, every one leaving the tree green:

1. Extract the frame step from `xr::run` and `desktop_participant::State::frame` into a module inside `tomori-quest`, with tests. This is the only step that changes behaviour-bearing code, and the one with the most value even if nothing else follows.
2. Move the renderers into `tomori-render`.
3. Move the app logic into `tomori-app`. `tomori-quest` is now the shell.
4. Add the exclusion metadata to `tomori-quest`, once `bake-test` supports it.

### Enforcing thinness

| Option | For | Against |
| --- | --- | --- |
| Review checklist only | No tooling | Relies on memory; the shell grew this way in the first place |
| `limit` in the metadata, set just above the shell's current size | Growth needs a visible `Cargo.toml` change and a reason in review; the summary shows the headroom | One more number to maintain; it's a cap on the exception, not a floor on coverage |
| Dependency rule: `tomori-app` and `tomori-render` never depend on `openxr`, `ash`, `winit`, `cpal`, `ndk` or `jni` | The compiler enforces the direction; already a validation step in `design/clients.md` | Doesn't limit logic written directly in the shell |
| A lint on the shell, such as limits on branching or function length | Automatic | Crude; branches on platform results are legitimate |

**Recommendation.** Use the dependency rule, which comes free with the crate split, plus `limit`, plus a written rule: the shell makes no decisions. A pull request that raises `limit` says why in its description.

This is not a floor. The covered crates stay at 100%; `limit` only stops the exception from growing.

## Staging

| Stage | What | Where | State |
| --- | --- | --- | --- |
| 1 | Run the ignored GPU tests on lavapipe in the coverage job, and fix or adapt the llvmpipe-only failure | Tomori | In progress. About 4,100 regions, reaching ~87.4% |
| 2 | Server-side tests: `tomori-server`, `tomori-backend-opencode`, `tomori-speech`, `tomori-discovery` and the small crates | Tomori | In progress. About 4,700 regions. The speech models need fetching and caching in CI (a question below). The multicast tests may run on a runner's loopback with multicast enabled. |
| 3 | `bake-test` 0.4: the exclusion metadata, measuring only the workspace, the summary; the guidance in `base-project-testing` and `context/testing.md` | base-rs `bake-test`, `base-project` | Proposed. Tomori then updates its `bake/` dependency |
| 4 | The `tomori-quest` split, in the four pull requests above, then the remaining app-logic tests | Tomori | Proposed. Lines up with #73's builders and `design/clients.md`. This is where most of the remaining `tomori-quest` gap closes |
| 5 | Make the gate required | Tomori | When the workspace passes. See below |

**Stage 5, making the gate required:**

- Move the coverage step into `test.yml` as Base's canonical workflow does, on stable only; the minimum-version matrix entry keeps plain `cargo test`.
- Add it to `test-result`'s `needs`, so that the ruleset already requiring `test-result` requires it.
- Delete `coverage.yml`, and its grep step if `bake-test` writes the job summary itself.
- Set a longer timeout than the current 30 minutes, since the lavapipe and model tests add time.

**A possible earlier step, not a floor.** Run `cargo bake test:coverage --package <crate>` as a required job for each crate as soon as it reaches 100% on its own tests, so that finished crates can't slip back while others catch up. Whether `--package` reports only that crate's files under cargo-llvm-cov needs checking, and it's stricter: other crates' tests don't count. Drop these jobs once the workspace gate is required.

## Design documents affected, if adopted

This proposal changes no `design/*.md` file. If it's agreed:

- **`design/crates.md`:** add `tomori-app` and `tomori-render` to the unpublished table, and record the coverage exclusion rule for `publish = false` crates.
- **`design/clients.md`:** name `tomori-app` as the headset's platform-neutral core, and add to its validation that the core and render crates depend on no OpenXR, windowing or platform audio crate. It already asks for most of this.
- **A testing requirement,** in a new `design/testing.md` or in `readme.md`'s Tests section:
  - 100% of measured regions;
  - the one excluded crate and its reason;
  - the "no decisions in the shell" rule.

## Open questions

1. **One mechanism.** Is a crate-level exclusion the only one Base offers, with no file lists, attributes or `cfg(coverage)`? And should exclusions be limited to `publish = false` crates?
2. **The limit.** Should `limit` exist, and should Tomori use it? Or is the summary's size line, plus review, enough?
3. **Names and shape.** Should `tomori-quest` stay the name of the shell, since it is the APK? What should the covered crate be called: `tomori-app`, `tomori-headset` or `tomori-participant`? Should `tomori-render` start now or wait for #97? Does the desktop window glue stay in the same shell, or go to its own `client-desktop` or `client-spectator` shell (#56)?
4. **Examples.** Keep them unmeasured and thin, as they are now, or run them as smoke tests? Should `recognition_benchmark.rs` become a bench or a binary?
5. **Speech models in CI.** Fetch and cache them, which costs time and cache space, so that the ignored model tests run? Or test the loaders against small fixtures and leave full models to manual runs?
6. **Test layout.** Move inline `mod tests` blocks into `tests.rs` files, as Base lays them out, which also takes test code out of the measurement?
7. **Device evidence.** Is an occasional manual coverage run on the headset worth setting up, or is a thin, reviewed shell enough? Is a Monado spike worth doing to shrink the shell further?
8. **Ordering.** Should the frame-step extraction (split step 1) come before the `bake-test` work, since it pays off on its own? And should crates get required per-package gates as they reach 100%?
