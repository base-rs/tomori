# Client Logging

## Requirement

The headset client's log keeps its lifecycle and user-meaningful events available for many minutes of use. On the headset it logs to Android logcat, under the tag `tomori`, at `info` and above.

- Lifecycle and user-meaningful events are logged at `info`, or at `warn` or `error` when something failed: XR session state changes, the headset being put on or taken off, engagement changes with the signal that caused them, muting and unmuting spoken replies, connecting to and joining a server or space, utterances failing, the microphone and speaker opening and closing, permission results, interaction profile changes, applied recenters, and each segment of agent speech with its text.
- The headset logs once at startup whether the runtime reports the haptic extensions `XR_FB_haptic_amplitude_envelope` and `XR_FB_haptic_pcm`. Each feedback event (see `feedback.md`) is logged at `debug`, and a failure to vibrate a controller at `warn` when it changes, with its recovery at `info`.
- Nothing is logged at `info` or above on every frame, audio callback or read. An event that can recur many times a minute is logged at `debug` or `trace`, logged only when it changes, or logged at `info` or `warn` at most once per interval of at least 30 seconds. Frame submission is logged once when the first frame is submitted and then summarized, with the frame count and rate, at most once a minute.
- When occurrences are skipped by such an interval, the next one logged says how many were skipped. A count is reported with the next logged occurrence, not on a timer, except that live microphone overruns still unreported when capture stops are reported then.
- A warning that a frame loop could repeat is logged when its condition changes, not on every frame.

## Rationale

In November 2026, per-frame `info` lines ("Submitted stereo frame", swarm interaction modes and repeated reference-space changes) filled the logcat buffer within about two minutes, erasing the connections, joins, utterances and errors needed to debug a problem on the headset. The user asked that per-frame and high-frequency lines be demoted or logged on a slow cadence, keeping lifecycle events at `info`.

## Scope

Logging in `crates/client-quest`, in particular its XR frame loop, input sampling, connection polling, and the audio device callbacks and capture loop. The desktop and spectator modes share this code and follow the same rule; the server's logging is covered by `server.md`.

## Validation

- Inspect each `info`, `warn` and `error` log call in `crates/client-quest` that can run from the frame loop, a polling loop or an audio callback, and check that it is guarded by a change of state or by a cadence of at least 30 seconds.
- The cadence is unit tested in `crates/client-quest/src/log_cadence.rs`: the first occurrence is logged, later ones are skipped and counted until the interval has passed, and the rate is reported.
- On the headset, run `adb logcat -s tomori` through several minutes of use, including joining a space, talking and recentering, and check that steady use adds only a summary line about once a minute while lifecycle events remain in the buffer.
