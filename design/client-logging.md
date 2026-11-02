# Client Logging

## Requirement

The headset client's log keeps its lifecycle and user-meaningful events available for many minutes of use. On the headset it logs to Android logcat, under the tag `tomori`, at `info` and above.

- Lifecycle and user-meaningful events are logged at `info`, or at `warn` or `error` when something failed: XR session state changes, connecting to and joining a server or space, utterances failing, the microphone and speaker opening and closing, permission results, interaction profile changes, and applied recenters.
- Nothing is logged at `info` or above on every frame. An event that can recur many times a minute is logged at `debug` or `trace`, logged only when it changes, or logged at `info` at most once per interval of at least 30 seconds. Frame submission is logged once when the first frame is submitted and then summarized, with the frame count and rate, at most once a minute.
- A warning that a frame loop could repeat is logged when its condition changes, not on every frame.

## Rationale

In November 2026, per-frame `info` lines ("Submitted stereo frame", swarm interaction modes and repeated reference-space changes) filled the logcat buffer within about two minutes, erasing the connections, joins, utterances and errors needed to debug a problem on the headset. The user asked that per-frame and high-frequency lines be demoted or logged on a slow cadence, keeping lifecycle events at `info`.

## Scope

Logging in `crates/client-quest`, in particular its XR frame loop, input sampling and connection polling. The desktop and spectator modes share this code and follow the same rule; the server's logging is covered by `server.md`.

## Validation

- Inspect each `info`, `warn` and `error` log call in `crates/client-quest` that can run from the frame loop or a polling loop, and check that it is guarded by a change of state or by a cadence of at least 30 seconds.
- The cadence is unit tested in `crates/client-quest/src/log_cadence.rs`: the first occurrence is logged, later ones are skipped and counted until the interval has passed, and the rate is reported.
- On the headset, run `adb logcat -s tomori` through several minutes of use, including joining a space, talking and recentering, and check that steady use adds only a summary line about once a minute while lifecycle events remain in the buffer.
