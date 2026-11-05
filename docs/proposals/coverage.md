# Proposal: Reaching 100% Region Coverage When Some Code Can't Run in CI

Status: **decided, and carried out in the pull requests below**, November 2026. Once they merge, the gate is required, with exclusions per file rather than per crate, as Bake's coverage task came to support them; see the outcome below. The rest of this document is the proposal as it was decided, kept for its reasoning. It covers [#55](https://github.com/ioquatix/tomori/issues/55) and touches stage 3 of [#73](https://github.com/ioquatix/tomori/issues/73), the server and headset builders.

## Outcome

`bake-test` 0.3.5 added the exclusion, as `[[package.metadata.bake.test.coverage.exclude]]` entries in a package's own `Cargo.toml`, one per file, each with a `path` and a required `reason`. The task fails on an entry without a reason, on a file that does not exist, and on a file listed twice. Its report always lists each excluded file with its reason and its size, in regions run and lines, whether the gate passes or fails. That answers decision 5, though per file rather than per crate. The other rules are as decided: exclusion is a last resort, the excluded code is thin, and the logic around it is covered.

A file is a finer boundary than a crate, so the headset did not have to be split for the gate. The device calls went into thin files of their own instead, and everything around them was tested. The split in #119 remains worth doing for its own reasons. The excluded files, as the gate's report lists them in CI:

| File | Regions | Run by tests | Lines the regions span |
| --- | ---: | ---: | ---: |
| `crates/client-quest/src/xr.rs` | 1,083 | 12 | 586 |
| `crates/client-quest/src/desktop_window.rs` | 277 | 14 | 167 |
| `crates/client-quest/src/vulkan.rs` | 244 | 0 | 187 |
| `crates/client-quest/src/desktop_audio.rs` | 145 | 0 | 85 |
| `crates/server/src/process.rs` | 59 | 49 | 52 |
| `crates/discovery/src/daemon.rs` | 32 | 23 | 19 |
| `crates/client-quest/src/child_process.rs` | 15 | 6 | 12 |

That is 1,855 regions, about 1.5% of the workspace, under the 3,600 this proposal estimated. Every one of the other 120,989 measured regions runs, and Bake leaves out the 161 inside `unreachable!`. With #137, the Test workflow runs the gate on stable Rust, the main branch requires it through `test-result`, `coverage.yml` is gone, and `design/testing.md` records the requirement.

The work is in [#130](https://github.com/ioquatix/tomori/pull/130) (Bake 0.3.5 and the exclusions), [#131](https://github.com/ioquatix/tomori/pull/131) (the headset), [#135](https://github.com/ioquatix/tomori/pull/135) (the server and discovery), [#136](https://github.com/ioquatix/tomori/pull/136) (the OpenCode backend, speech, client and bake) and [#137](https://github.com/ioquatix/tomori/pull/137) (making the gate required). Reaching 100% found these bugs, each fixed with a regression test:
- a Markdown hard break and a double space after a task marker;
- `tomori discover` running a space's backend and its participant count together;
- the pause between spoken segments following only a segment that produced no audio;
- a message shorter than the truncation marker being replaced by it;
- a list emptied to fit a snapshot or listing being a byte over budget.

Unreachable branches were removed, or became `expect` or `unreachable!` with their reasons. Branches that only a race could take were restructured so that coverage does not depend on timing.

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

## Decisions

From discussion with the maintainer, on this proposal's pull request:

1. **Any crate may be excluded, published or not,** with a required, non-empty reason. The coverage report always names each excluded crate and shows its size. The headset shell will probably be published, because #73 stage 3 turns it into a builder that other projects use.
2. **There is no size cap,** but exclusion is a last resort, not a convenience. A crate is excluded only where testing it is genuinely impractical in CI: it needs a headset, a display or audio hardware. The default answer to uncovered code is to write tests.
3. **Crate names** come from the environment framework ([#97](https://github.com/ioquatix/tomori/issues/97)): `tomori-scene` for the shared scene state and frame step, and `tomori-render` for the renderers.
4. **The frame step** shares only what is genuinely common between the headset and the desktop window, such as turning input into state, the conversation and the shared panels. It doesn't force one UX: the desktop and Quest interfaces may diverge. Platform-specific behaviour lives in covered, per-platform logic modules behind a narrow interface. Only the raw device calls stay in the shell.
5. **Not decided:** changing Bake's coverage task, in base-rs's `bake-test`, to read the exclusion metadata. The headset's restructuring goes ahead first, since it pays off on its own.

## Recommendation in brief

- **One exclusion mechanism: a whole crate.** Add `[package.metadata.bake.test.coverage]` to `bake-test`, with `exclude = true` and a required, non-empty `reason`.
  - Any workspace crate may use it, published or not.
  - The excluded crate's tests still run and must pass. Only its regions stop counting.
  - The summary always names each excluded crate, its reason, its size in regions and how much of it the tests reach anyway. The exclusion can't quietly grow.
  - No size limit, no file regexes, no `#[coverage(off)]`, no `cfg(coverage)` stubs.
- **Measure only the workspace.** `bake-test` should count only regions under the workspace root. Today it fails on hosts whose standard library sources live outside rustup, such as Arch's `/usr/src/debug/rust`.
- **Make the excluded crate thin by splitting `tomori-quest`.**
  - A platform shell keeps the `tomori-quest` name, the APK, the binary and every OpenXR, Vulkan-interop, Android, winit, cpal and PipeWire call. It is excluded.
  - Everything else moves to covered crates: the scene state and frame step to `tomori-scene`, with the #73 builder, and the renderers to `tomori-render`, tested on lavapipe.
  - The key change is a shared frame step for what the headset and the desktop window have in common, with each platform's own behaviour in a covered module of its own. Today the frame logic is written twice, inside `xr::run` and `desktop_participant::State::frame`, and neither copy is tested.
- **Keep it thin with two checks:**
  - a review rule: the shell makes no decisions;
  - the dependency direction: the covered crates never depend on a platform crate.
- **Examples stay unmeasured,** as cargo-llvm-cov already arranges. Anything an example needs beyond argument parsing and printing belongs in a library, where it's covered.
- **Device and self-hosted runs** can provide evidence, but they don't count toward the gate.

## What the numbers say

### Now

Re-measured on `origin/main` at `3960402` on 2026-11-04, with `cargo llvm-cov --workspace --all-targets` and `cargo llvm-cov report --ignore-filename-regex '^/usr/src/'`, all tests passing, on a host with a GPU:

| | Regions | Uncovered | Covered | Share of the gap |
| --- | ---: | ---: | ---: | ---: |
| Workspace | 100,236 | 9,364 | 90.7% | 100% |
| `tomori-quest` (`crates/client-quest`) | 50,515 | 7,116 | 85.9% | 76.0% |
| `tomori-server` | 19,641 | 1,110 | 94.3% | 11.9% |
| `tomori-speech` | 5,431 | 653 | 88.0% | 7.0% |
| `tomori-backend-opencode` | 14,499 | 350 | 97.6% | 3.7% |
| Others | 10,150 | 135 | 98.7% | 1.4% |

`tomori-speech`'s figure is lower here than in CI, which fetches Whisper tiny.en for its model tests; this host has only small.en.

Since the first measurement, at 82.9%, the rendering tests run on lavapipe in CI, the server, OpenCode backend, client, protocol, security and discovery crates have been tested, and CI fetches and caches the speech models. Three quarters of what remains is in `tomori-quest`:

| File | Regions | Uncovered | Covered |
| --- | ---: | ---: | ---: |
| `xr.rs` | 2,570 | 1,864 | 27.5% |
| `desktop_participant.rs` | 1,077 | 916 | 15.0% |
| `connection.rs` | 3,367 | 777 | 76.9% |
| `desktop.rs` | 699 | 500 | 28.5% |
| `console.rs` | 425 | 377 | 11.3% |
| `conversation.rs` | 10,048 | 358 | 96.4% |
| `controller_ui.rs` | 1,099 | 270 | 75.4% |
| `vulkan.rs` | 244 | 244 | 0% |
| `main.rs` | 165 | 165 | 0% |
| `pipewire_video.rs` | 336 | 155 | 53.9% |
| `spectator_feed.rs` | 428 | 154 | 64.0% |
| `discover.rs` | 126 | 126 | 0% |
| `capture.rs`, `playback.rs` | 1,014 | 233 | 77.0% |
| Others | 28,917 | 977 | 96.6% |

### The first measurement

From the analysis that started this proposal, on 2026-11-04 at `fd2e9ea`, the workspace was at 82.9%. It sorted uncovered functions into categories:

| Category | Share | Where | Can CI run it? |
| --- | ---: | --- | --- |
| OpenXR and platform glue | 20% | `xr.rs`, `desktop*.rs`, `vulkan.rs`, `pipewire_video.rs` | Partly. Most of `xr::run` is interaction logic, not OpenXR calls. |
| Bodies of ignored GPU tests | 19% | inline test modules in `graphics.rs` and `workspace_graphics.rs` | Yes, on lavapipe. Done. |
| Networking and error paths | 18% | both `connection.rs` files, `hub.rs`, `wire.rs`, `api.rs` | Yes. The server side is done. |
| GPU and rendering | 12% | renderers, `environment-sky` | Yes, on lavapipe. Done. |
| Examples | 7% | `recognition_benchmark.rs`, `presence_sender.rs`, `connect.rs` | Not measured today. |
| CLI and `main` | 7% | `tomori-server`'s `main.rs`, `console.rs`, `discover.rs` | Yes, with injected arguments and I/O. The server's is done. |
| Untested pure logic | 5% | `conversation.rs`, `transcript.rs`, `markdown_lite.rs` | Yes. |
| Speech models and their ignored tests | 7% | `kokoro.rs`, `transducer.rs`, `vad.rs`, `offline.rs` | Yes. CI now fetches and caches the models. |
| Test helpers' unused branches | 3% | various | Yes, or remove the branches. |
| Audio devices, multicast tests | 2% | `capture.rs`, `playback.rs`, `discovery` | Partly. Multicast on loopback is done. |

### Two things the totals hide

- **Android-only code is never measured.** About 675 lines under `cfg(target_os = "android")` aren't even compiled by a host coverage run:
  - `android.rs`;
  - the Android parts of `lib.rs`: `android_main`, `AndroidEventPump` and the Android connection;
  - the AAudio modules of `capture.rs` and `playback.rs`.

  Nothing reports them, so they're the silent erosion this proposal wants to prevent.
- **Some code exists only for Android.** `capture.rs`'s `run_capture`, `drain` and `InputStream`, and `graphics::requested_samples`, are compiled on the host but called only from Android code, so the host build flags them as dead.

### What is truly beyond GitHub CI

About 3,600 regions:

- the OpenXR calls in `xr::run`;
- `vulkan.rs`, the Vulkan device that OpenXR requires;
- the winit event handlers;
- PipeWire output.

That's under 4% of the workspace. An exclusion sized to that, and no bigger, is the goal. It isn't a cap; it's what decision 2 implies once everything testable is tested.

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
  - An excluded crate could grow unnoticed if the summary only said "excluded". So the summary always reports its size.
- **Verdict:** decided. It is the only exclusion mechanism.

### 2. File or module exclusion via `--ignore-filename-regex` from config

A workspace setting, such as `ignore = ["crates/client-quest/src/xr\\.rs"]`, is passed to cargo-llvm-cov.

- **For:** fine-grained, and needs no restructuring.
- **Against:**
  - It invites excluding mixed files whole: `xr.rs` is mostly interaction logic, and excluding it would hide about 1,000 regions of decisions.
  - Regexes also match files added later.
  - They go stale silently when files move.
  - A module boundary is weaker than a crate's: nothing stops covered code calling into an excluded file, or excluded code growing logic.
- **Verdict:** not recommended.

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
  - Most of `xr::run` decides what buttons, rays, palms and holds mean: hold-to-leave, which wrist wins, when to tick haptics, where panels stand.
  - `desktop_participant::State::frame` repeats much of it for the keyboard and mouse. Its comments often say "as on the headset" or "see `xr::run`".
  - A shared frame step for the common part, with plain input, removes the duplication and makes all of it testable. The Base testing skill warns against "production abstractions solely to satisfy coverage"; this one also removes a real duplication.
- **(b) A real OpenXR runtime in CI.**
  - Monado, the open-source runtime, has a null compositor and simulated devices.
  - If it runs headless on a GitHub runner with lavapipe, the real `run_xr` path, which already exists for host runtimes, could run as a test, and the shell could shrink further.
  - Whether it does needs a spike.
- **Verdict:** do (a), started first; see the staging. Try (b) later; if it works, it shrinks the exclusion, and the metadata stays the same.

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
- Moving Tomori's inline test modules into `tests.rs` files would make the layout consistent.

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

Not decided: this is what `bake-test` would read if the change goes ahead. The metadata goes in the excluded crate's own manifest, under the existing `bake.test` namespace:

```toml
# crates/client-quest/Cargo.toml
[package.metadata.bake.test.coverage]
exclude = true
reason = "OpenXR session and frame loop, the Vulkan device OpenXR requires, the winit event handlers, and the Android activity, AAudio and JNI glue: they need a headset, a display or audio hardware that GitHub CI lacks. Logic lives in tomori-scene and tomori-render, which are covered."
```

**Schema.** These rules are enforced by `bake-test`, and an error names the manifest:

| Key | Type | Rule |
| --- | --- | --- |
| `exclude` | boolean | Required if the table is present. `false` is allowed, so an exclusion can be switched off without deleting its history. |
| `reason` | string | Required and non-empty, after trimming, when `exclude = true`. It should say what can't run and why, and where the logic went. |
| anything else | | Rejected, so a typo such as `reasons` fails rather than being ignored. |

There is no size limit (decision 2). The summary's size line, and review, keep the exclusion honest.

**Where the table may appear:**

- Only in a workspace member's `[package.metadata]`, published or not (decision 1).
- `[workspace.metadata.bake.test.coverage]` is reserved for workspace-wide settings and has no `exclude`.

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

The summary keeps exclusions in view whether the gate passes or fails. It always names the excluded crate and its size. The figures below are illustrative:

```text
Region coverage passed: 96,630/96,630 measured source regions covered.
  Excluded by unreachable!(): 3 regions.
  Excluded crate tomori-quest (crates/client-quest): 3,480 regions, 1,104 reached by tests (31.7%).
    Reason: OpenXR session and frame loop, the Vulkan device OpenXR requires, the winit event handlers, …
  Not measured (cargo-llvm-cov defaults): examples/, tests/, benches/, tests.rs files.
```

When the gate fails, the same lines follow the count of uncovered regions, then the region list, one per line. `bake-test` could also append the first few lines to `$GITHUB_STEP_SUMMARY` when that is set, so Tomori's grep step can go. That part is a separate change.

### Base guidance

The `base-project-testing` skill and `bake-test`'s `context/testing.md` would gain a short section:

- **A last resort.** A crate may be excluded only when testing it in CI is genuinely impractical: its code needs hardware CI doesn't have, such as a headset, a display or audio devices. The default answer to uncovered code is to write tests.
- **Never for:**
  - code that is merely hard to test: injected I/O, fakes and `#[cfg(test)]` shims come first, as the skill already says;
  - logic.
- **Published or not.** A published crate may be excluded, as a headset shell that others build on is. Its reason must say what can't run.
- **The excluded crate is a shell.** It translates between platform types and plain types, and makes no decisions. Any branch that isn't about a platform call's result belongs in a covered crate.
- **Platform-specific code.** Code compiled only for a platform the gate doesn't run on, such as `cfg(target_os = "android")`, lives in the excluded crate, or is tested on that platform. This replaces the skill's "run coverage on each supported target" for targets CI can't run tests on.
- **Reviewing a change to an exclusion.** It needs the same scrutiny as lowering a target would. Reviewers check the reason, the size and whether logic moved in.

## Keeping the excluded crate thin: splitting `tomori-quest`

### The shape

```text
tomori-quest   (crates/client-quest; excluded; the APK, the `tomori` binary, the cdylib; probably published, as #73's headset builder)
  ├── tomori-scene   (crates/scene; covered; the scene state, the shared frame step, each platform's logic; the builder's core)
  ├── tomori-render  (crates/render; covered on lavapipe; wgpu renderers)
  └── openxr, ash, android-activity, ndk, jni, winit, cpal
tomori-render ──► tomori-scene's plain types, tomori-environment-sky, wgpu
tomori-scene  ──► tomori-client, tomori-protocol, tomori-security, tomori-discovery, glam
```

**Names.** `tomori-scene` and `tomori-render` are the names `docs/naming.md` already plans for #97 (decision 3). They can start here as the headset's scene logic and renderers, and grow into the environment framework.

**Why this shape:**

- **It's what `design/clients.md` already requires:**
  - a platform-neutral core and scene model "free of rendering code";
  - rendering "in its own crate, not in one client that others depend on";
  - engagement derived from platform-neutral signals.

  The split brings the code in line with that design.
- **It's what #73 needs.** A downstream headset build registers its environments through a builder and uses `tomori-quest`'s shell to run, without forking either.
- **It keeps names stable.** `tomori-quest` keeps its name and its `[package.metadata.android]`, so `scripts/android.sh`, the APK name and the application ID don't change.

### The seam: a shared frame step and per-platform logic

The headset and the desktop window share some of what they do each frame, but not all of it, and their interfaces may diverge (decision 4). So the seam has three parts:

- **The shared step,** for what is genuinely common, given plain input:
  - the controller's buttons as one frame's presses and holds: navigating, the right wrist's transcript, B and holding it to leave, Y and the pinned hand menu, A, push-to-talk, muting and replaying;
  - letting go of everything held when focus is lost;
  - arrival's motion, the hand menu, the right wrist's transcript, pointing and choosing;
  - the swarm, the space's workers and result cards;
  - the shared panels: the status pill, the recording indicator and the transcript;
  - what to draw, where the participant hears from, and the presence to send.
- **Each platform's logic,** in a covered module of its own, behind a narrow interface:
  - the headset: which hand's palm shows what from controller poses and hand tracking, the pinned menu above the left controller, the live microphone's grip, the waiting cues on the palms, the drawn ray, vibrations and the session's focus;
  - the desktop window: the keys and mouse as the controller's buttons, the head the mouse turns, the palms held in view while M or T is down, knocks in the title, and which feedback it gives without controllers.
- **The shell,** with only raw device calls: OpenXR's session, actions, spaces and swapchain, the Vulkan device, winit's events and surface, and the audio devices. It fills the input from the device and carries out the output.

The tests drive the shared step and each platform's logic with scripted input. For example: "hold B in a space for 1 s and the participant leaves", "the right palm within 45° of up for 0.2 s shows the transcript", "a focus loss during X cancels the utterance".

### What moves where

This is a high-level, file-by-file plan. "Split" means part of the file stays as glue and the rest moves.

| File | Today | Goes to |
| --- | --- | --- |
| `main.rs`, `android.rs` | entry points, Android activity | **shell** |
| `lib.rs` | module list, `run_*` entry points, `android_main`, `AndroidEventPump` | **shell**: the entry points, which call into `tomori-scene` |
| `xr.rs` | `run`, the session loop and frame body; `Input`, actions and bindings; `HandTracking`; `ControllerModels`; `SceneAnchor` and view maths | **split**: the session, swapchain, action and space calls stay in the **shell**. Into **scene**: the frame body's decisions, `Controls::from_actions`, `LiveGrip`, `SceneAnchor`'s maths, `initial_scene_transform`, `head_pose` and `view_projection` (with `glam` input), and `arrival_target` |
| `vulkan.rs` | the Vulkan instance and device for OpenXR, image import | **shell** |
| `haptics.rs` | OpenXR haptic action | **split**: the action stays in the **shell**; which pulse for which event goes to **scene**, next to `feedback.rs` |
| `desktop.rs` | winit app, window GPU, `Camera` | **split**: the `ApplicationHandler`, surface and acquisition stay in the **shell**; `Camera` and its ray maths go to **scene**, with their tests |
| `desktop_participant.rs` | winit app, key mapping, `Head`, `State::frame` | **split**: the event handler stays in the **shell**; the key mapping, `Head` and the desktop's own frame logic go to **scene**, using the shared step |
| `headless_spectator.rs` | PipeWire loop | **split**: `next_frame_time` and the draw schedule go to **scene**; the loop stays in the **shell** |
| `pipewire_video.rs` | GPU readback, gstreamer child | **split**: `pack_rows` and the frame layout go to **render**; `Producer` stays in the **shell** (it is already partly tested through `from_child`) |
| `capture.rs` | `Capture` trait, downmix, cpal and AAudio modules, `run_capture`, `drain`, `InputStream` | **split**: the trait, `downmix`, `run_capture`, `drain` and `InputStream` go to **scene**, tested on the host with a fake stream, which also ends their dead-code warnings; the cpal and AAudio modules stay in the **shell** |
| `playback.rs` | `Playback` trait, channel rendering, cpal and AAudio modules | **split** the same way |
| `graphics.rs`, `workspace_graphics.rs`, `arrival_graphics.rs`, `controller_graphics.rs`, `ui_panel.rs`, `spectator_view.rs` and the `.wgsl` files | wgpu renderers | **render**, covered on lavapipe |
| `controller_ui.rs` | the shared panels' logic and their GPU panels | **split**: what shows, where and with what text goes to **scene**; the panels to **render** |
| `ui_text.rs`, `caption.rs`, `swarm.rs` | text layout, captions, the swarm simulation | **scene** if free of wgpu, otherwise **render** |
| `conversation.rs`, `connection.rs`, `delegation.rs`, `transcript.rs`, `markdown_lite.rs`, `hand_menu.rs`, `arrival.rs`, `arrival_motion.rs`, `controller.rs`, `interaction.rs`, `pointing.rs`, `panel_anchor.rs`, `pill.rs`, `recording.rs`, `feedback.rs`, `spatial_audio.rs`, `speakers.rs`, `server_choice.rs`, `identity.rs`, `log_cadence.rs`, `environment.rs`, `hands.rs` | app logic and state | **scene**. Most are already above 95%. `connection.rs`, with 777 uncovered regions, and `conversation.rs`, with 358, are the main gaps to test. |
| `spectator.rs`, `spectator_feed.rs` | the spectator's options and feed | **scene** for now; `crates/client-spectator` later, per `design/clients.md` and #56 |
| `console.rs`, `discover.rs` | terminal panel, server listing | **scene**, with their input and output injected so that `run` is tested; they need no platform |

**Estimated shell size.** Roughly 2,000–3,500 measured regions:

- the OpenXR calls left in `xr.rs`;
- `vulkan.rs` (244);
- the winit handlers;
- device opening in `capture.rs` and `playback.rs`;
- the gstreamer `Producer`;
- the entry points;
- the Android code.

That is within the ~3,600 regions the analysis found truly beyond CI, and about 3% of the workspace.

**Order of the split.** One pull request each, every one leaving the tree green:

1. Draw the device boundary inside `tomori-quest`: extract the shared frame step and each platform's logic from `xr::run` and `desktop_participant::State::frame` into covered modules, with tests, leaving the device files as thin calls. This is the only step that changes behaviour-bearing code, and the one with the most value even if nothing else follows. Started in [#119](https://github.com/ioquatix/tomori/pull/119).
2. Move the renderers into `tomori-render`.
3. Move the scene logic into `tomori-scene`. `tomori-quest` is now the shell.
4. Add the exclusion metadata to `tomori-quest`, if `bake-test` supports it by then (decision 5).

### Enforcing thinness

| Option | For | Against |
| --- | --- | --- |
| Review rule: the shell makes no decisions | No tooling | Relies on memory; the shell grew this way in the first place |
| The summary's size line for each excluded crate | Every run shows the shell's size and how much tests reach; growth shows in review | Nothing fails on its own |
| Dependency rule: `tomori-scene` and `tomori-render` never depend on `openxr`, `ash`, `winit`, `cpal`, `ndk` or `jni` | The compiler enforces the direction; already a validation step in `design/clients.md` | Doesn't limit logic written directly in the shell |
| A lint on the shell, such as limits on branching or function length | Automatic | Crude; branches on platform results are legitimate |

**Decided.** No size limit (decision 2). Use the dependency rule, which comes free with the crate split, the review rule, and the summary's size line.

## Staging

| Stage | What | Where | State |
| --- | --- | --- | --- |
| 1 | Run the GPU tests on lavapipe in the coverage job | Tomori | Done |
| 2 | Server-side tests: `tomori-server`, `tomori-backend-opencode`, `tomori-speech`, `tomori-discovery` and the small crates, with the speech models fetched and cached in CI | Tomori | Mostly done; the workspace is at about 91% |
| 3 | `bake-test` 0.4: the exclusion metadata, measuring only the workspace, the summary; the guidance in `base-project-testing` and `context/testing.md` | base-rs `bake-test`, `base-project` | Not decided (decision 5) |
| 4 | The `tomori-quest` split, in the four pull requests above, then the remaining tests of the scene logic | Tomori | The first pull request is [#119](https://github.com/ioquatix/tomori/pull/119). Lines up with #73's builders and `design/clients.md`. This is where most of the remaining `tomori-quest` gap closes |
| 5 | Make the gate required | Tomori | When the workspace passes. See below |

**Stage 5, making the gate required:**

- Move the coverage step into `test.yml` as Base's canonical workflow does, on stable only; the minimum-version matrix entry keeps plain `cargo test`.
- Add it to `test-result`'s `needs`, so that the ruleset already requiring `test-result` requires it.
- Delete `coverage.yml`, and its grep step if `bake-test` writes the job summary itself.
- Keep its timeout above the current run time; the lavapipe and model tests add time.

**A possible earlier step, not a floor.** Run `cargo bake test:coverage --package <crate>` as a required job for each crate as soon as it reaches 100% on its own tests, so that finished crates can't slip back while others catch up. `tomori-protocol`, `tomori-security` and `tomori-backend` were at 100% in the measurement above. Whether `--package` reports only that crate's files under cargo-llvm-cov needs checking, and it's stricter: other crates' tests don't count. Drop these jobs once the workspace gate is required.

## Design documents affected, if adopted

This proposal changes no `design/*.md` file. As it's carried out:

- **`design/crates.md`:** add `tomori-scene` and `tomori-render`, and say whether each is published; record that any crate may carry a coverage exclusion, with a reason, as a last resort.
- **`design/clients.md`:** name `tomori-scene` as the headset's and desktop window's platform-neutral core, with the shared frame step and each platform's logic, and add to its validation that the scene and render crates depend on no OpenXR, windowing or platform audio crate. It already asks for most of this.
- **A testing requirement,** in a new `design/testing.md` or in `readme.md`'s Tests section:
  - 100% of measured regions;
  - the one excluded crate and its reason;
  - the "no decisions in the shell" rule.

## Open questions

Answered: the mechanism and who may use it (decision 1), the limit (decision 2), the covered crates' names (decision 3), the frame step's shape (decision 4), fetching the speech models in CI (done), and the ordering: the headset's restructuring comes first, without waiting for Bake (decision 5).

Still open:

1. **The Bake change.** Should `bake-test` read the exclusion metadata as described, and when?
2. **The shell's name and the desktop window.** Should `tomori-quest` stay the name of the shell, since it is the APK? Does the desktop window's glue stay in the same shell, or go to its own `client-desktop` or `client-spectator` shell (#56)?
3. **Examples.** Keep them unmeasured and thin, as they are now, or run them as smoke tests? Should `recognition_benchmark.rs` become a bench or a binary?
4. **Test layout.** Move inline `mod tests` blocks into `tests.rs` files, as Base lays them out, which also takes test code out of the measurement?
5. **Device evidence.** Is an occasional manual coverage run on the headset worth setting up, or is a thin, reviewed shell enough? Is a Monado spike worth doing to shrink the shell further?
6. **Per-crate gates.** Should crates get required per-package gates as they reach 100%?
