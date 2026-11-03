# Proposal: Audio and Haptics as One Feedback Design

Status: **stage 1 agreed and implemented**, November 2026. The user's decisions are recorded under [Decisions](#decisions). Stage 1 is now a requirement in `design/feedback.md` (sound cues and vibration) and `design/spatial-audio.md` (spatial playback, ducking and the first step of echo cancellation), with changes to `design/live-audio.md`, `design/spaces.md` and others, which take precedence over this document. Stages 2 and 3 remain proposals for discussion.

## The request

Spoken from the headset, November 2026: "We need to also design the audio." The user asked for:

- **Spatial audio.** The agent's voice comes from the swarm, participants' voices from their avatars, and workers' updates from their stations.
- **Sound cues (earcons).** Recording start and stop, sent, an approval waiting, attention, and catching up on return.
- **Mixing.**
  - Ducking the agent under people.
  - Volume per person.
  - Echo cancellation for the live microphone, which `design/live-audio.md` deferred as best-effort with a feedback warning.
- **Voices.** Whether sub-agents get voices of their own, and how environment audio fits in, such as window audio played from panels in the planned computer environment (#88).
- **Haptics.** "Do the Quest controllers support vibration? Is that part of the audio design, like pointing could vibrate when you point over something?"

The short answer to the last question is yes. Quest controllers vibrate through OpenXR's haptic output action, and Meta adds richer waveforms through two extensions. The controllers have no force feedback: a vibration can say "something happened here", but it cannot resist the hand. This proposal treats sound and vibration as one vocabulary of feedback.

## Recommendation in brief

- **Spatialize on the client, with simple panning.** The server keeps sending mono streams per source, as it does today. Each client places each source in its own scene:
  - the agent at the swarm;
  - each participant at their head, taken from presence;
  - workers at their stations;
  - environment sources at their panels.

  The client then renders stereo with equal-power panning, a small interaural delay, distance attenuation with a floor, and gain smoothing. This costs almost nothing on the Quest. HRTF can come later, as an option.

- **One feedback vocabulary of about a dozen events.** Each event pairs a short procedurally synthesized earcon with a controller vibration, on the hand that acted. Examples are hovering onto something, choosing it, recording, sent, failed, a request waiting, catching up, a knock, and the live microphone. Everything else stays silent. A **Feedback** setting chooses full, reduced (requests and failures only) or off, for sound and haptics separately.

- **Mixing rules.**
  - The agent ducks about 8 dB while a person speaks.
  - Environment audio ducks under any speech.
  - Each participant gets a local volume and mute.
  - Received voices are levelled gently, and the existing limiter stays last.

- **Echo cancellation in two steps.**
  - **Step A (MVP):** use the platform's canceller properly. Today the speaker opens with `AudioUsage::Media` and no shared audio session (`crates/client-quest/src/playback.rs:197`). Step A changes that, logs whether Android reports an `AcousticEchoCanceler`, and measures echo with a repeatable test.
  - **Step B:** only if step A falls short on the Quest, run WebRTC's AEC3 on the client. It would use `webrtc-audio-processing` with the exact mix the player renders as its reference. The pure-Rust `decibri-aec` would be measured alongside it.

- **Voices.**
  - The main agent keeps `af_heart`.
  - Workers stay silent until they have something short to say, such as finishing or needing approval. Then the server speaks a short announcement, positioned at the worker's station, in a voice from a small palette.
  - Announcements go into the space's one speech stream between blocks, so that two voices never talk at once.

- **Environment audio** (#84, #88) enters the same mixer as positioned sources, under the same ducking, volume and echo rules.

- **MVP is client-only, with no protocol change.** It covers:
  1. stereo spatial playback;
  2. the haptic action and core vibrations;
  3. the earcons;
  4. ducking;
  5. echo cancellation step A, with its measurement.

  Worker voices need one additive field on `ServerMessage::Speech`, in stage 2.

## 1. Current state and its problems

Everything below is from `origin/main` at `a55e9bb`.

### How sources are mixed

- **One mono mix.**
  - `client::playback::Mixer` (`crates/client/src/playback/mixer.rs`) gives each received source its own jitter buffer. The sources are participants by number, `AGENT_SOURCE` and `REPLAY_SOURCE`.
  - It sums the sources at unity gain into **one mono `i16` stream** at the device rate, then a peak limiter (`playback/limiter.rs`, ceiling 32 000, 100 ms release) converts it.
  - The only local gains are the agent's volume and mute (`set_agent_volume`, `set_agent_muted`), which also apply to replays.
  - Each source's level is measured before its gain, for the speaking indicator (`source_level`).
- **Mono output on every device.**
  - **Quest:** the speaker opens AAudio as **mono**: `channel_count(1)`, `AudioUsage::Media`, `AudioContentType::Speech`, `LowLatency`, shared mode (`crates/client-quest/src/playback.rs:192–200`).
  - **Desktop:** cpal copies the mono sample into every channel (`playback.rs`, `desktop::build`).
  - **Spectator:** the spectator plays through the same `Desktop` output (`crates/client-quest/src/spectator_feed.rs:131`), so its audience hears the same mono mix.
- **No server-side spectator mix.** `crates/server/src/mixer.rs` is a port of the bridge's mixer at 16 kHz, but it is "not yet wired to a spectator feed" (line 4). `design/live-audio.md` says the server mixes for the spectator feed; in practice that mix happens in the spectator client.
- **Nothing is positioned.**
  - The Quest does not even consume presence. The presence hook is `#[cfg(not(target_os = "android"))]` (`crates/client-quest/src/connection.rs:587`), and only the spectator draws participants.
  - Presence carries the head, both controllers and hand joints (`crates/protocol/src/message.rs:127`, `presence.rs`). The desktop reports the head only (`design/desktop.md`).
  - The agent's position is a client constant, `swarm::CENTER = (0, 1.4, -2.3)` (`crates/client-quest/src/swarm.rs:8`).
  - Worker stations are laid out on the client (`delegation.rs`), and the spectator lays them out the same way.

### Formats and latency

- **Formats.**
  - Participant audio is 20 ms frames of 16-bit mono PCM. The Quest captures at 16 kHz (`capture.rs:335`), and the desktop at its device rate, clamped to 8–48 kHz.
  - Agent speech is Kokoro at 24 kHz (`crates/speech/src/tts.rs:184`). The pacer sends it 100 ms ahead of its playing time (`crates/server/src/retention.rs:30`), with a 400 ms breath between segments (`SEGMENT_GAP`).
  - Everything travels over the one HTTP streaming connection. `design/server.md` plans UDP and Opus later.
- **Latency.**
  - The jitter buffer starts each source at 60 ms and caps it at 250 ms (`jitter_buffer.rs:27–33`). It conceals up to three missing frames with silence and then skips.
  - Mouth to ear on a LAN should be roughly 20 ms of capture, plus the network, plus 60 ms of jitter buffer, plus the device's output buffer: around 100–150 ms. Nobody has measured it, and `design/live-audio.md` ends with "Latency targets … are not yet agreed."

### Echo

- **Push-to-talk.**
  - Push-to-talk captures with `VoiceRecognition` (`capture.rs:375`), which has no echo cancellation.
  - Playback is therefore silenced during push-to-talk, with a 10 ms fade (`player.rs`, `FADE`). That works, but it means you cannot hear the agent or anyone else while you hold X.
- **The live microphone.**
  - Live capture requests `VoiceCommunication` (`capture.rs:387`).
  - Android normally attaches its echo canceller to that input preset. The canceller works best when playback is also a voice-communication stream, ideally in the same audio session.
  - Tomori's output is `Media`, with no session id linking the two streams.
  - Whether the Quest 3's `VoiceCommunication` preset cancels anything has not been verified. `design/live-audio.md` asks for a two-headset feedback test "before relying on echo cancellation", and that test has not been done.
- **The desktop** has no echo cancellation at all.

### Haptics and cues

- **No haptic output action exists.**
  - The action set (`crates/client-quest/src/xr.rs:1117–1192`) creates pose, float, bool and vector actions only.
  - The startup log reports render model, hand tracking, simultaneous hands and user presence extensions (`xr.rs:41–60`), but not the haptic ones.

- **The openxr crate supports haptics.** The crate is `openxr 0.22` (`crates/client-quest/Cargo.toml:32`). It has:
  - `Action<Haptic>::apply_feedback` and `stop_feedback`;
  - `HapticVibration`, with duration, frequency and amplitude;
  - `Duration::MIN_HAPTIC`, the shortest pulse the device supports;
  - builders for `HapticAmplitudeEnvelopeVibrationFB` and `HapticPcmVibrationFB`;
  - the `fb_haptic_amplitude_envelope` and `fb_haptic_pcm` extension flags.

  Meta documents both extensions for its Touch controllers. Whether this runtime reports them is not yet known, because Tomori doesn't log them. Stage 1 adds that log line.

- **The cues are visual only.** Feedback today is the panel's text, the swarm's colour (`interaction.rs`), the hover highlight of the pointing ray (`pointing.rs`), and the wrist glow for a knock (`hand_menu::cue`, `hand_menu.rs:497`).

### Problems, in short

1. **Everything sounds as if it comes from the middle of your head.** You cannot tell the agent from a person by direction, and several speakers mask each other.
2. **No per-person control.** Only the agent can be muted or turned down.
3. **Nothing is levelled.** A quiet microphone stays quiet, and a loud one only meets the limiter.
4. **The live microphone feeds back on speakers.** The canceller is unverified and probably not engaged.
5. **Nothing you hear or feel says a request is waiting.** Approvals are silent, as the controller UI proposal notes (PR #89, "spoken replies are heard but approvals are silent").
6. **Workers make no sound.** Only the space's own agent's assistant text is spoken (`crates/server/src/voice.rs:269`).

## 2. Spatial audio

### Where to spatialize: the client

The server should not spatialize:

- each listener needs their own mix;
- the mix must follow their head with no network delay;
- head poses arrive at frame rate, and a round trip of 50–150 ms would make sources swim as the head turns.

The client already knows everything it needs: its own head pose each frame, the other participants' heads from relayed presence, and the swarm, stations and panels in its own scene. So:

- **The server sends mono per source, unchanged.** The frame format and sources stay as they are.
- **The client places each source.**

| Source | Position | When unknown |
| --- | --- | --- |
| `AGENT_SOURCE`, `REPLAY_SOURCE` | The swarm's animated centre, starting at `swarm::CENTER` | `swarm::CENTER` |
| A participant | Their head, from their latest relayed presence, offset 10 cm down to the mouth | A seat on a ring around the swarm, by public handle (see open questions) |
| A worker's announcement (stage 2) | That worker's station anchor | The swarm |
| An environment source (stage 3) | Its panel or entity, as an extended source | Unpositioned |
| Earcons | Head-relative: slightly towards the hand that acted, or at the swarm for "the agent needs you" | Centre |

- **The Quest starts consuming presence** for audio, before it draws avatars. This lifts the `cfg` on `Connection::watch_presence` and keeps the latest head pose per source.
- **A listener pose and a snapshot of source positions** go to the audio callback each frame, through a single latest-wins slot such as a small triple buffer. The existing command channel queues every message, which would build up at frame rate. The player interpolates gains across each callback, so that nothing clicks.

### How: panning, not HRTF, first

Per source, in the listener's head frame:

- **Direction.** Equal-power panning by azimuth, never fully silencing the far ear: at least about −10 dB. The Quest 3's speakers sit on the strap and are open to both ears, and hard panning on open speakers sounds wrong.
- **Interaural delay.** Up to 0.6 ms, as a fractional delay on the far ear. This makes left and right much clearer for speech, at the cost of two taps per sample.
- **Behind the head.** A gentle one-pole low-pass on the far ear for sources behind and to the side, so that front and back are not identical.
- **Distance.** Inverse-distance gain referenced to 1 m and clamped below 0.5 m. Speech is never attenuated more than about 12 dB, and the agent never more than 6 dB, so distance changes presence but never intelligibility.
- **Smoothing.** Gains and delays ramp over one callback (about 5–10 ms).

**Cost.** This is a handful of multiply-adds per source per sample. Eight sources at 48 kHz stereo is well under 1% of one Quest 3 core.

**HRTF later, as an option.** Convolving each source with a measured head-related impulse response gives real elevation and front–back cues, especially on headphones.

- **Candidates:**
  - the `hrtf` crate (pure Rust, MIT, IRCAM sphere data);
  - Steam Audio (Apache-2.0, a C library).
- **Why not first:** each costs one partitioned convolution per source, about 1–2% of a core each.
- **Rejected:** Meta's XR Audio SDK is native and under Meta's own licence, so it is not proposed.

HRTF matters most for headphones. Most Quest users hear the strap speakers, where panning plus delay already does most of the work.

**Platform constraints.**

- **Quest.** AAudio must open with two channels. The shared-mode mixer accepts stereo at the device rate, normally 48 kHz.
- **Echo.** Stereo playback makes the echo reference stereo, so a software canceller (section 5) is fed the stereo mix downmixed to mono, which is what the single microphone hears.
- **Desktop.** cpal already writes every channel; it would write the left and right channels instead.
- **Accessibility.** A **mono** setting turns panning off, for people who hear with one ear, while keeping distance and ducking.

### Spectator, desktop and console

- **Spectator.** The listener is the spectator's camera: a fixed or following pose, the one it renders from. The output is stereo, so that an OBS audience hears the agent where they see the swarm. The spectator plays no earcons, consistent with `design/spectator.md`, which shows no HUD.
- **Desktop.** The listener is the desktop participant's head, at arrival's home, turned by right-dragging. A desktop has no haptics, so earcons carry every cue.
- **Console.** Mono and unpositioned, as today.
- **The server's `mixer.rs`** stays unwired. If a server-side spectator or recording feed is wanted later, it can mix with the same fixed positions as the spectator.

## 3. Earcons and a haptic vocabulary

### Principles

- **Few, distinct and paired.** Each event has one sound and one vibration with the same rhythm, such as one tap or two. A participant learns the rhythm once, and it works through either channel.

- **On the hand that acted.**
  - Pointing, choosing, the wrist transcript and leaving use the **right** controller.
  - Recording, the live microphone and the hand menu's knock use the **left**.

  This matches the controller UI proposal: the recording card sits on the left controller, and the transcript and requests on the right wrist.

- **Feedback for state changes, not for state.** A cue marks the moment something changes. Nothing hums continuously, and the visual stays the record of the current state.

- **Never the only channel.** Every cue duplicates something already visible: the panel or status pill, the recording card, a highlight, or the wrist glow. Hand tracking has no haptics, and the desktop has no haptics, so sound carries the cue there.

- **Quieter than speech.** Earcons sit about 12 dB below speech, are short (40–250 ms), and come from one small family of pitches, a pentatonic set, so that they sound related and never clash with each other.

### The vocabulary

Haptic values are for the core `XrHapticVibration`: amplitude from 0 to 1, duration, and frequency left unspecified. "MIN" is `Duration::MIN_HAPTIC`.

| Event | When exactly | Sound | Haptic | Hand |
| --- | --- | --- | --- | --- |
| **Hover** | The ray enters a new target: a worker, an arrival object, a hand menu item or a card button. Not on leaving, not while pulling the swarm | None by default; an optional faint tick | One tick, MIN at 0.2, at most one per 80 ms | Right |
| **Choose** | The trigger chooses, or A confirms a choice, mode or cancellation | Soft click | One tap, 20 ms at 0.5 | Right |
| **Recording starts** | X is pressed and the gate opens | Short rising "on" tick, 40 ms (see below) | One tap, 25 ms at 0.6 | Left |
| **Recording stops** | X is released and the utterance ends | None; "sent" follows | One light tap, 15 ms at 0.35 | Left |
| **Sent** | The server queues or answers the utterance | Two rising notes | None, since the stop tap was just felt | — |
| **Not sent / failed** | An utterance fails, is too long, or a send fails | Two falling notes, low | Two taps, 30 ms at 0.7, 80 ms apart | The hand that acted |
| **Limit warning** | 30 s and 10 s before the utterance limit (PR #89, stage 2) | None | One tap at 30 s, two at 10 s | Left |
| **Request waiting** | An approval or question arrives for the space | Three-note chime, positioned at the swarm | Three taps, 25 ms at 0.6 | Right, where the transcript shows requests |
| **Request reminder** | The same request is still waiting after 60 s and its card has not been opened | The chime, quieter | One tap | Right |
| **Catching up** | Speech played again starts after joining or returning (a `Speech` with `replay: true` when no replay was asked for) | A soft "rewind" swell, from the swarm | None | — |
| **Knock** | A pending knock appears (`design/hand-menu.md`) | Two soft knocks | Two taps on the left wrist | Left |
| **Live microphone on/off** | The left grip toggles live | "On": the rising tick. "Off": the falling tick | On: one tap. Off: two light taps | Left |
| **Holding to leave** | B is held towards the one-second leave | None until it fires; then a low click | A ramp from 0.1 to 0.6 over the second, then a tap when it fires | Right |
| **Attention elsewhere** (`docs/proposals/attention.md`, stage 3) | Another space or server reaches *input* | A distant version of the request chime | None | — |

**Recording start, push-to-talk and silencing.**

- **The conflict.** Playback is silenced before the microphone opens for push-to-talk, so any sound played then would either be silenced or be captured.
- **The proposal.**
  - The start tick bypasses the silence. It is 40 ms long and played at X's press, while the capture stream is still opening.
  - The tick must never be transcribed. The test plan checks that it isn't, on the Quest and on a desktop.
  - If it does reach the transcript, the Quest falls back to haptics only for "recording starts", and the desktop plays the tick and opens the microphone after it.
- **Other earcons** follow the normal silencing during push-to-talk, except "not sent".

### What not to signal

- **Speech is its own signal.** There is no cue for the agent starting to speak, for messages streaming, partial transcripts, tool calls, worker state changes, or activity changes.
- **Leaving a target, or staying on one.** There is no repeated buzz while hovering.
- **Brief connection blips.** A reconnection in under about 5 seconds gets no cue. A lost connection shows on the panel; a sound for it could come later.
- **Presence changes**, such as someone taking their headset off or joining. Joining could earn a cue later if people want it.
- **Anything for visitors' actions.**
- **Spectator output.** It plays no earcons at all.

### Settings and accessibility

- **The Feedback setting.**
  - Two choices: **Sounds** (full, reduced or off) and **Vibration** (full, reduced or off).
  - **Reduced** keeps only requests, failures and the knock.
  - It lives in the hand menu, in the Space section the controller UI proposal adds, or in a small **Sound** section of its own. The console has `feedback sounds|vibration full|reduced|off`.
  - It is kept on the device, not in the space.
- **Earcon volume** is relative to speech, and is not a separate master volume.
- **Mono audio** (section 2) and **no panning of earcons** in mono mode.
- **Accessibility.** Cues differ by rhythm and pitch direction, not only by timbre, so they stay distinct on poor speakers and for people with reduced pitch discrimination. Haptics can be turned off for people sensitive to vibration.

### Format, licensing and implementation

- **Synthesized in Rust; there are no sound files.**
  - Each earcon is a few sine or FM partials with an attack–decay envelope, described in a table: pitch, duration, envelope, partials and level.
  - It is rendered once into a buffer at the device rate when the player's mixer is created. The audio callback then only copies samples, without allocating.
  - There are no assets to license or package, nothing to ship in the APK, and the output is deterministic for tests.
  - If the user later wants designed sounds, the table can name CC0 samples instead.
- **Where the code goes.**
  - A `playback::Earcon` source in `crates/client`, mixed after ducking and before the limiter. It is a local source: never sent, never counted as a speaker, and in the echo canceller's reference.
  - A `haptics` module in `crates/client-quest`, pure and tested. It maps a feedback event to a pattern, and keeps a small per-hand sequencer that the frame loop ticks.
  - A multi-pulse pattern uses several `apply_feedback` calls, one per frame boundary, with the core API. With `XR_FB_haptic_amplitude_envelope` it becomes one envelope call.
- **One haptic action**, `haptic`, created with the subaction paths `/user/hand/left` and `/user/hand/right`. It is bound to `/user/hand/{left,right}/output/haptic` in the `oculus/touch_controller` profile. This is the same action the controller UI proposal plans for its stage 2, so the two proposals share it rather than adding two.
- **Logging** follows `design/client-logging.md`.
  - The haptic extensions are logged once at startup.
  - Feedback events are logged at `debug`.
  - A haptic call that fails is logged when the failure changes.

## 4. Mixing

The client's mixer, in order:

1. Each source's jitter buffer, as today.
2. **Levelling** (stage 2), on participants only.
   - A slow automatic gain of about 3 s towards a speech level of about −20 dBFS RMS.
   - At most +12 dB of boost and −12 dB of cut, held while the source is silent so that noise is not pumped up.
   - Agent speech is levelled once, at the server, after synthesis. Kokoro's level is consistent, so it is measured and a fixed gain is applied, matching it to the participants' target.
3. **Per-source gain.** The participant's local volume, and the agent's volume and mute, as today.
4. **Ducking.**
   - **The agent ducks under people.** While any participant source plays above a speech threshold for 100 ms, agent speech (live and replayed) eases down by about 8 dB, over 80 ms down and 500 ms back up. The threshold uses the existing `source_level`.
   - **Ducking is not stopping.** Ducking is local and the stream continues, so `design/speech.md`'s "agent speech is not interrupted" holds.
   - **Environment audio ducks under any speech**, people or agent, by about 12 dB.
   - **Earcons never duck anything, and are never ducked.**
5. **Spatialization** to stereo (section 2).
6. **Earcons**, head-relative.
7. **The silencing fade during push-to-talk**, as today, with the start-tick exception above.
8. **The limiter**, as today.

**Priority.** People talking with each other come first, then the agent, then environment audio. Earcons sit beside them all, short and quiet. Speaking indicators keep measuring before gain, as `design/live-audio.md` requires.

**Per-participant volume and mute** (stage 2):

- **Where.** Each participant's row in the hand menu's People & security section gains mute and three volume steps (50%, 100%, 150%). The desktop reaches them the same way; the console has `volume NAME PERCENT` and `mute NAME`.
- **Local only.** The setting is local, as `design/spaces.md` allows ("local presentation choices such as mute").
- **Keyed by public handle,** which stays stable while the server keeps the participant. It is mapped to their audio source number, and applies across reconnects.
- **The speaking indicator** still shows a muted person speaking, because it is measured before gain.
- **Co-located participants.** Muting is also the remedy when two participants share a physical room, where each hears the other acoustically and through the speakers at once.

## 5. Echo cancellation for the live microphone on Quest

### Option A: the platform canceller, used properly (recommended first)

Android attaches its acoustic echo canceller to `VoiceCommunication` capture, where the device has one. For it to work well:

- **Output as communication.** Play through `AudioUsage::VoiceCommunication` while the live microphone is open, rather than `Media`. The usage could switch when live opens, or apply whenever connected (an open question).
- **One audio session.** Allocate an AAudio session id on the output stream and give the same id to the live input stream. The `ndk 0.9` crate in use has `AudioStreamBuilder::session_id`.
- **Ask, and log.** Use `AcousticEchoCanceler.isAvailable()`, and `create(sessionId)` and `getEnabled()` through JNI (`crates/client-quest/src/android.rs` already calls Java), and log the result once. This turns "may provide echo cancellation" into a fact for each device.
- **Measure** (see the test strategy below).

**Cost:** none on the CPU, since it runs in the audio HAL or DSP. **Licensing:** none. **Risks:**

- The communication usage may change routing and volume, for example to the voice-call volume stream.
- It may add processing such as automatic gain or noise suppression.
- The Quest 3 may not implement echo cancellation at all.

### Option B: a software canceller in the client

The client has an advantage most apps lack: it renders the exact signal that goes to the speaker, in `Player::render`. So the far-end reference is available in-process, sample-accurate apart from the device's output and input latency, which a canceller's delay estimator absorbs.

| Option | What it is | Quality | Cost on Quest 3 (estimated) | Licence | Integration |
| --- | --- | --- | --- | --- | --- |
| **WebRTC AEC3** via `webrtc-audio-processing` 2.x (tonarino) | Bindings to PulseAudio and PipeWire's packaging of WebRTC's audio processing module: AEC3, noise suppression, AGC2 | The reference standard for conferencing; robust delay estimation and double-talk handling | A few percent of one core at 16 kHz mono, in 10 ms frames | BSD-3 (WebRTC) and MIT (bindings); Abseil under Apache-2.0 | C++ through meson with the `bundled` feature. It needs the Android NDK C++ toolchain and Abseil, so it is real build work for the APK. Others have built it for Android ABIs |
| **speexdsp** (MDF canceller) | The classic C library | Adequate for linear echo; weak on double-talk and on loudspeaker distortion | Below 1% | BSD-style | Small C, easy to cross-compile; Rust bindings exist |
| **`decibri-aec`** 0.2 | A new pure-Rust canceller with its own delay acquisition | Reports AECMOS gains on the ICASSP challenge set, but only by its author; unproven elsewhere | Unknown; to be measured | Apache-2.0 | Pure Rust, no cross-compilation work. It is young, with one maintainer |

**Placement.**

- Run the canceller on the capture thread, at 16 kHz.
- Feed it the player's mix as its reference: downmixed to mono, resampled to 16 kHz, and passed through a lock-free ring from the output callback.
- Apply it only to `live` frames. Push-to-talk keeps silencing playback and the recognition preset, so recognition never depends on a canceller's residue.
- Desktop live capture would get the same canceller, which covers the desktop's missing echo cancellation as well.

### Recommendation

1. **Do option A in the MVP.** It is cheap, may be all that is needed, and the logging and measurement are needed anyway.
2. **Build the measurement harness at the same time.** It plays a known speech file through the client's own player while capturing with the live preset, and saves both. It then computes echo return loss enhancement (ERLE) offline.
   - On a desktop, it runs from a test binary.
   - On the Quest, it runs from a debug intent extra, `tomori.echo_test`, run by a person when the headset is free.
3. **If the platform gives less than about 20 dB of ERLE on the Quest's speakers, take option B with WebRTC AEC3.** Measure `decibri-aec` in the same harness first, because a pure-Rust canceller that is close enough would avoid the C++ build.
4. **Keep the feedback warning** in `design/live-audio.md` until the measurement passes. Then replace it with the measured result and the device it was measured on.
5. **Later, and separately:** with a verified canceller, push-to-talk could duck playback instead of silencing it, so you still hear the room while talking to the agent. That changes a decision the user made for recognition integrity, so it is only listed as an open question.

## 6. Voices

**Today** there is one voice, `af_heart` (Kokoro speaker 3, `crates/speech/src/tts/kokoro.rs:65`). It speaks the space's own agent's assistant text and nothing else. Kokoro v1.0 holds 54 voices in one `voices.bin`, so another voice is only a different speaker id in a request. It needs no memory and no second engine.

**The proposal:**

- **The main agent keeps `af_heart`, exclusively.** Its voice is the one participants know.
- **Workers don't narrate.** Speaking workers' streaming output would flood the space. Instead, the server speaks **announcements**: one short sentence when a worker finishes, fails or needs an approval or answer, such as "Explorer finished." or "Reviewer needs your approval." The worker is named by the label the swarm shows.
- **Announcements come from the station.** Each one is positioned at the worker's station, so the voice says which worker it is by where it comes from.
- **A small palette of distinct voices** for workers, chosen for contrast with `af_heart` and with each other, such as two British and two American voices of both genders, picked by ear. Each worker gets one by a stable hash of its identifier.
- **Optional voices.** Whether workers get distinct voices at all is the user's choice. Position alone may be enough.
- **Scheduling: one speech stream, one voice at a time.**
  - Announcements are queued on the space's existing speech pacer.
  - They are inserted at the next block boundary, in the 400 ms breath, never mid-block.
  - An announcement more than 10 seconds stale is dropped, because the panel shows it anyway.
  - Several are merged: "Two workers finished."
  - They are kept and replayed with the reply they interrupted, like any other speech, so a catch-up hears them too.
  - Two voices overlapping is much harder to follow than a short wait, so they never overlap.
- **Brief speech** (`design/speech.md`) does not cut announcements, which are always one sentence.
- **Protocol** (stage 2):
  - `ServerMessage::Speech` gains an optional `speaker: Option<Speaker>`, `{ worker: String }`, with `#[serde(default)]`. Older clients ignore it.
  - The client positions `AGENT_SOURCE`, or `REPLAY_SOURCE`, at that worker's station for that segment.
  - No new audio source is needed, and the catch-up and replay logic is unchanged. Positions switch during the breath, so the previous segment's tail does not jump.

## 7. Environment audio

The environment framework (PR #90) gives environments **media streams** (#84). The computer environment plays each window's audio from its panel (#88). They fit the mixer as follows:

- **Sources of their own.**
  - Environment audio arrives as media streams: Opus, with timestamps on the shared clock. It never travels as `AudioFrame` participant sources.
  - It is decoded off the audio thread and pushed into the mixer as **environment sources**, keyed by stream.
  - Their jitter target is longer than speech's, and set to match the video presentation delay for lip sync, rather than the 60 ms used for speech.
- **Positions** come from environment state: the panel's pose. A panel is an **extended source**: within its width, panning is spread rather than pinned to its centre.
- **The same rules apply.**
  - Environment audio ducks under people and the agent.
  - Each stream gets a local volume and mute, as #88 asks.
  - It is part of the echo canceller's reference, because it is part of the mix.
  - The spectator includes it.
- **Feedback.** The server half never captures Tomori's own output on the computer, such as the spectator feed or agent speech, as #88 requires. Participants' microphones are never routed into the computer.
- **What this proposal adds.** #88's "positioned sources and spatial mixing in `crates/client`" task is the MVP of this proposal, so #88 only needs to add a source kind.

## 8. Staging

### MVP (stage 1): spatial playback, haptics and cues, client only

1. **Stereo spatial playback.**
   - **Output:** stereo on the Quest (AAudio, two channels) and the desktop.
   - **Positions:** the Quest consumes presence for head positions; participants are positioned at their heads, and the agent and replays at the swarm.
   - **Panning:** panning, interaural delay, distance and smoothing.
   - **Other clients and settings:** the spectator listens from its camera, and there is a mono setting.
2. **Haptics.**
   - **The action:** the haptic output action on both hands, with the FB haptic extensions logged at startup.
   - **The vocabulary:** hover, choose, recording start and stop, failure, request waiting and its reminder, knock, live on and off, and holding to leave.
3. **Earcons.** Procedural sound for the same events plus "sent" and "catching up", and the Feedback setting (sounds and vibration, each full, reduced or off) in the hand menu and console.
4. **Ducking.** The agent under people, at about 8 dB.
5. **Echo cancellation, step A.**
   - Communication usage and a shared session id while live.
   - `AcousticEchoCanceler` availability logged.
   - The echo measurement harness.

**No protocol or server change.** It aligns with the controller UI work: it uses the same haptic action, the same hands for the same interface, and the same request and recording moments.

### Stage 2: people, workers and a better canceller

- Per-participant volume and mute in the hand menu and console, and participant levelling. Agent speech is levelled at the server.
- Worker announcements at stations, optionally in palette voices; `Speech.speaker` (additive).
- Option B, WebRTC AEC3 or `decibri-aec`, if the step A measurement falls short; desktop live capture gets it too.
- Amplitude-envelope haptics where `XR_FB_haptic_amplitude_envelope` is reported, and the controller UI's 30 s and 10 s limit pulses.

### Stage 3: environments and richer sound

- Environment sources (#84, #88) in the mixer, as extended positioned sources with lip-sync delay.
- An HRTF option for headphones.
- An attention chime for other spaces and servers (attention proposal, stage 3).
- PCM haptic textures, if useful, such as a subtle texture while dragging a panel.
- Possibly ducking instead of silencing during push-to-talk, once a canceller is verified.

## Server and protocol changes

| Stage | Change |
| --- | --- |
| MVP | None |
| 2 | `ServerMessage::Speech.speaker: Option<Speaker>` (additive). Worker announcements in `crates/server/src/voice.rs`'s pacer, with a speaker id per segment through `speech::Speech`. A fixed level gain on synthesized speech |
| 3 | Environment media streams (#84), designed there; this proposal only consumes them |

## Test strategy

**Unit tests** (in `crates/client` and `crates/client-quest`, under the 100% region coverage rule):

- **Panner.**
  - A source on the left is louder and earlier in the left channel.
  - The far ear never falls below its floor.
  - Distance gain is clamped near and far.
  - Gains ramp without steps larger than a bound.
  - Mono mode produces identical channels.
- **The mixer renders stereo.** A `Player` with two sources at known positions and a listener renders each in its side.
- **Ducking.** The agent ducks only while a participant plays above the threshold, follows the attack and release, and still reports its level before gain.
- **Earcons.**
  - Each renders deterministically, peaks at the earcon level, and allocates nothing in `mix`, being pre-rendered.
  - Earcons are never treated as a speaker.
  - They are silenced during push-to-talk, except the start tick and "not sent".
- **Haptics.**
  - The event-to-pattern mapping and the hand for each event.
  - At most one hover tick per 80 ms, and none on leaving or while pulling.
  - The request reminder fires once, after 60 s, unless the card was opened.
  - The sequencer emits multi-pulse patterns at frame boundaries.
  - The Feedback setting filters events in each mode.
- **Per-participant volume** follows a participant's handle across a reconnect that keeps their source.
- **Announcements** (stage 2) are inserted only at block boundaries, dropped when stale, merged, kept for catch-up, and positioned by `Speech.speaker`.

**On a desktop:**

- **Echo harness.** Speaker-to-microphone ERLE with and without a canceller.
- **CPU cost.** The player's render time with eight positioned sources.

**On the headset** (manual, by a person, when the headset is free):

- The agent sounds as if it comes from the swarm, and a second participant from where they stand.
- Turning the head moves the sound, without clicks.
- Hover ticks feel light and do not chatter when sweeping across several workers.
- The request chime and taps are noticed while looking away.
- The start tick does not reach the transcript.
- Logcat shows the haptic extensions and `AcousticEchoCanceler` availability.
- The two-headset feedback test from `design/live-audio.md`, with and without step A.

## Risks

- **Communication usage changes the Quest's audio routing or volume.** *Mitigation:* switch the usage only while live, and measure the volume range. If it is unacceptable, keep `Media` and rely on option B.
- **There is no platform echo canceller on the Quest.** *Mitigation:* option B, prepared by the harness.
- **The WebRTC build in the APK** brings C++, meson and Abseil to the Android build. *Mitigation:* measure `decibri-aec` first, and add the build only if needed.
- **Cue fatigue.** *Mitigation:* the short list, "what not to signal", the reduced mode, and tuning on the headset.
- **Spatial cues on open speakers** are weaker than on headphones, and hard panning sounds odd. *Mitigation:* the floor on the far ear, and the mono option.
- **Remote participants' positions.** Remote participants' presence is relative to their own home, so two people at home would sound as if they stand where the listener stands. *Mitigation:* seats (see open questions), which avatars will need as well.
- **Overlap with the controller UI implementation** (branch `controller-ui`). *Mitigation:* one haptic action, the same hand assignments and the same event moments. Whichever lands second wires its events into the other's vocabulary.

## Decisions

The user answered questions 2, 3, 4, 6 and 7 in November 2026, and asked for stage 1 to be implemented; the others follow this document's recommendations. The numbers are those of the questions.

1. **The MVP scope** is stage 1 as proposed, in coherent parts: haptics and sound cues first, then spatial mixing.
2. **Hover vibration** is on by default, but very light. Hovering makes no sound.
3. **Remote participants**, who share no physical space with the listener, are placed around a circle at the edge of the space, so that they never sound as if they stand inside the listener.
4. **Workers** may speak short updates later, in the agent's voice for now. This is not part of stage 1.
5. **Communication usage** follows the recommendation: the speaker switches to it only while the live microphone is on.
6. **Echo:** measure first; no bar is set yet. The feedback warning in `design/live-audio.md` stays until measurements justify dropping it.
7. **Push-to-talk** is to duck other sound rather than silence it, but only once echo cancellation is verified. Until then it silences playback as before.
8. **Sounds** are procedurally synthesized, as proposed.

## Open questions for the user

These were the questions put to the user; their answers are under [Decisions](#decisions).

1. **The MVP scope.** Is the MVP right: spatial playback, haptics, earcons, ducking and echo cancellation step A, with per-person volume in stage 2? Or should per-person volume come first?
2. **Hover vibration.** On by default (a light tick) or off by default? And should hovering also make a faint sound?
3. **Where remote people sit.** Should remote participants have seats around the swarm (a ring ordered by join), with their head movements applied relative to the seat? This affects avatars too, and probably belongs in `design/spaces.md`.
4. **Worker voices.** Should workers speak short announcements from their stations at all? If so, in the agent's voice or in distinct voices?
5. **Communication usage.** Switch the speaker to communication usage only while the live microphone is on, or always?
6. **The bar for echo cancellation.** Is about 20 dB of measured echo reduction the right bar for dropping the feedback warning?
7. **Push-to-talk silencing.** Once a canceller is verified, would you rather keep hearing the room, ducked, while holding X, or keep the silence?
8. **Sounds.** Are procedurally synthesized earcons acceptable, or do you want designed sounds (CC0 samples)?

## Changes to the design documents

If this is agreed, the binding documents would change as follows. None are changed in this proposal.

- **`design/live-audio.md`:**
  - Positioned sources and the client's stereo spatial mix replace "a single output".
  - Ducking, the mixing order, and per-participant volume and mute.
  - The echo cancellation steps and measurement, replacing the warning once measured.
  - Earcons as local sources, and the start tick's exception to silencing.
  - Environment sources (with #88).
- **`design/speech.md`:** worker announcements, their scheduling at block boundaries, staleness and merging; worker voices if chosen; the server's level gain on synthesized speech.
- **`design/interaction.md`:** the feedback vocabulary (sounds and vibrations per event, and the hand for each), hover ticks, holding to leave with a haptic ramp, and the Feedback setting.
- **`design/hand-menu.md`:**
  - The knock's haptic and sound, beside the wrist cue.
  - The Feedback setting.
  - Mute and volume on each participant row.
- **`design/spectator.md`:** stereo from the camera's pose, with no earcons.
- **`design/desktop.md`:** stereo from the desktop head; earcons in place of haptics.
- **`design/client-logging.md`:** haptic extension support and `AcousticEchoCanceler` availability logged once; feedback events at `debug`.
- **`design/spaces.md`:** seats for remote participants, if chosen (question 3).
