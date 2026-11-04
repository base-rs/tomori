# Tomori

A conversational spatial workspace for people and agents, currently a native **Rust + OpenXR + wgpu/Vulkan** prototype for **Meta Quest 3**. 128 agents inhabit a moonlit Sky environment or a minimal floor grid. Real stereo views use the headset's predicted poses and asymmetric eye projections. The Quest renders the interactive scene and joins conversations through a Tomori server on the PC, which runs the agent sessions.

Tomori is the name of both the system and its conversational coordinator; see [docs/naming.md](docs/naming.md). The headset and desktop application is the Rust package `tomori-quest`, whose library and executable are named `tomori`, and the server is `tomori-server`; the other crates are listed under [Crates](#crates). The headset finds servers with Bonjour or over USB; several participants can share a space. Hold **X** to talk to the space's agent: the server transcribes what you say and queues it, and the agent's replies are spoken. Sky is an early environment blockout, not finished artwork. Private workspace memory and session state are not included in this public source repository.

- **Right trigger:** outside a space, choose what the right controller's ray points at; a pull that starts away from every object, or any pull inside a space, attracts the swarm to the right controller.
- **A:** connect, join a space, or confirm an approval; otherwise reset the swarm.
- **Hold Meta button:** Quest's system recenter brings the swarm in front of you. A short press opens the system menu. No artificial camera movement.

## Environments

Each space is bound to one environment, chosen when it is created and the same for everyone in it; an owner can change it later, for everyone, from the hand menu's **Space** section (see [design/environments.md](design/environments.md)). The server offers **Sky**, the default for a new space: a fixed moonlit night, sparse stars, static cloud banks, a stable platform and an open shrine blockout with warm lanterns. The concrete renderer lives in [`crates/environment-sky`](crates/environment-sky/readme.md), separate from sessions, worker simulation and application input. It also offers **Grid**, the plain floor grid of the **Minimal** environment, which is also the waiting area outside any space.

- **Quest / desktop XR:** choosing a server and a space happens on the grid; joining a space switches to its environment, and leaving returns to the grid. Inside a space, only an owner's change for the whole space switches it. A space whose environment this build doesn't know shows the grid, and its panel names the environment.
- **Desktop window:** it has no server, so press **E** to switch between Sky and the grid (with `--desktop` and a server, **E** does nothing: the space chooses); or launch with `TOMORI_ENVIRONMENT=minimal cargo run -p tomori-quest` to start on the grid.
- **Spectator:** shows the environment of the space it watches, as everyone in it does, and follows an owner's change to it.

Switching does not reset agents, conversations or worker positions. Sky uses the same scene/recenter transform as the workspace; it does not move the camera. Clouds use a bounded low-resolution volumetric density pass with translucent edges and approximate moonlighting; lanterns have brighter emissive faces and cast warm, occluded illumination onto the nearby static scenery through a build-time lightmap, not runtime point-light shadow maps. The same lightmap adds local ambient occlusion at contacts. The shrine and platform cast/receive softly filtered moon shadows from a cached static shadow map. Agent/status visuals and labels are not shadowed. There is no day/night cycle, scenery collision or new locomotion system.

The scene has been exercised on Quest 3 and tested offscreen in Vulkan; the on-device frame-time budget still needs measurement. The stereo/switch/recenter regression runs with the other tests (see [Tests](#tests--desktop-openxr)); to save a preview sheet:

```sh
TOMORI_SKY_PREVIEW=/tmp/opencode/tomori-sky.png cargo test --locked -p tomori-quest \
  sky_environment_renders_stereo_switches_and_preserves_recenter
```

## Join a space on a Tomori server

Start the server on the PC with the OpenCode backend (see [design/server.md](design/server.md)), which drives a running `opencode serve`: it finds one started with `opencode serve --service`, or the one `--opencode-url` or `OPENCODE_URL` names. OpenCode can use Claude models, so there is no separate Claude Code backend. Spaces may run in the directory the server starts in or beneath it; give `--workspace-root PATH`, repeated as needed, to allow other directories instead. The server links the sherpa-onnx speech libraries, so fetch and verify them once before the first build:

```sh
bash scripts/fetch-sherpa-onnx.sh
opencode serve --service &
cargo run -p tomori-server --bin tomori-server -- --backend opencode
```

Anyone who can reach the server can watch its spaces as a visitor. Creating spaces, talking to the agent and approving its tools need a token (see [design/security.md](design/security.md)). The easiest way to get one is to pair: choose the gold key beside the connected server in the headset, and it shows a 6-digit code. If the server's console shows the same code, type it there to confirm, or confirm it on the status page; the headset then keeps an owner token for that server. A server started with `--no-visitors` still admits a device that asks to pair.

You can also make an owner token on the PC, and give it to the headset or the desktop client once; each keeps it for that server:

```sh
cargo run -p tomori-server --bin tomori-server -- auth mint --owner --name "Sam's Quest"
TOMORI_TOKEN=tomori_… bash scripts/android.sh run     # or: tomori --xr --server HOST:8765 --token tomori_…
```

A token given this way goes only to the server given with `TOMORI_SERVER` or `--server`, or over USB, never to a server found on the network.

`tomori-server auth list` lists the tokens and `auth revoke ID` revokes one. The server keeps its tokens and TLS certificate in `~/.local/state/tomori` (`--state-directory` changes it), serves only TLS, and shows owners its status page at `https://<server>:8765/`, where they sign in with a token. Pass `--no-visitors` to refuse anyone without a token, or `--auth open` to let anyone in as an owner, as before.

The headset creates spaces with its **New space** object (see [Controls](#controls)), shown to owners. To create one from the PC instead, use the example client with an owner token: give it the server's address, then type `create` with the backend and a directory beneath a workspace root.

```sh
TOMORI_TOKEN=tomori_… cargo run -p tomori-client --example connect -- 127.0.0.1:8765
create opencode /home/sam/project
```

The space stays on the server after the example quits.

It advertises itself with Bonjour, so the headset lists it when both are on the same network. Over USB, `bash scripts/android.sh run` forwards the headset's port 8765 to the PC with `adb reverse`, and the list's **USB** entry connects through it; set `TOMORI_SERVER_PORT` when the server listens on another port. To skip the list, give an address: `TOMORI_SERVER=192.168.1.10:8765 bash scripts/android.sh run`, or pass the intent extras `tomori.server` and `tomori.name` to `am start`. Your name defaults to the headset's model name. The headset remembers the last server it connected to, and connects to it again when it finds it, unless you move the selection first. The headset draws with 4× multisample anti-aliasing; pass the intent extra `-e tomori.msaa off` to `am start` to compare without it.

### Controls

Before you join a space, servers, spaces and the **New space** object stand in the world in front of you, each labelled. Point the right controller at one: its ray turns cyan and the object is highlighted, with its full label above the row. Pull the right trigger to choose it. Choosing happens once per pull, and only for a pull that starts on an object, so a pull that starts elsewhere still attracts the swarm, even if the ray then crosses an object. The stick and buttons do the same, and the object they select is underlined.

- **Servers:** point and pull, or flick the left stick up/down and press **A**, to connect; **Y** searches again. A server with an incompatible protocol is shown, grey, with its version, but can't be chosen.
- **Spaces:** the spaces and the **New space** object are shown together; with no spaces, only **New space** is. Choose a space to join it. Choose the connected server, on the left, or press **B**, to return to the servers; **B** first dismisses an error or a note that a space has ended. **Y** lists again.
- **New space:** choosing it opens rings of choices around it, one at a time: the backend, then the folder, then, for a backend that can attach, a new session or one of its existing sessions in that folder. **B**, or choosing the centre object again, steps back to the previous ring. **Y** lists the sessions again.
- **In a space:** the panel stands in front of you and stays put while you look around; turn or walk away for a second and it glides back in front of you. Flick the left stick up/down to page through text and left/right to move between messages. Problems have their own line, marked **!** with how long ago they happened, and stay until **B** dismisses them or a newer one replaces them. The queue has its own strip at the bottom: flick the right stick up/down to select an entry, anyone's, and press **A** to cancel it; **A** again within 3 seconds undoes that.
- **Y:** open the list of the agent's modes, with their descriptions, and the space's speech setting (brief or full); choose one with the left stick and **A**. **Y** or **B** closes it. A space with neither fetches its state again.
- **Right stick click:** play the latest spoken reply again from its start, to you only; click again to stop it and follow live speech. A reply that starts meanwhile stops it, and plays live.
- **Left stick click:** mute or unmute spoken replies for you only; the status line shows **Muted**. Other participants still play.
- **Pending approval:** flick the right stick left/right to choose *allow once*, a standing option or *deny*, then press **A**. Nothing is decided without a press, and **B** never decides.
- **Pending question:** hold **X** and say your answer, or its number; it goes to the question rather than the queue. A question Tomori can't present says to answer it on the PC.
- **B:** close the list, deselect a queue entry, or dismiss the problem shown. It never denies, cancels or leaves. **Hold B** for a second to leave the space.
- **Hold X:** talk to the agent; release to send. See [Voice](#voice).

The server keeps the conversation: after a dropped connection the headset reconnects and rejoins the space by itself. If the space is gone, as after the server restarts, the headset returns to the space list.

### Delegated workers and result cards

The headset doesn't show workers or result cards for now: the PC bridge provided them, and they return through the server.

The coordinator can use OpenCode's existing subagent tool. Child sessions of the open root conversation appear as smaller swarms: cubes travel from the coordinator or parent worker to an automatically placed station. Stations form a stable arc around the observer's reference position, with inward-facing signposts. Existing history appears settled rather than masquerading as newly delegated work. On observed completion a worker settles into one solid, slowly rotating cube; on error, separate fragments with a gentle wave. Their labels and symbols show observed running, waiting, idle, completed, error, or unknown states; stale or absent data is not treated as completion. Partial observation errors do not erase independently observed worker statuses. This is a visual projection of actual OpenCode sessions, not a simulator or invented progress.

The coordinator can explicitly publish a reviewed result, linked to a source session and message. Workers do not need spatial interfaces and their outputs are not automatically displayed. Use `node scripts/workspace.mjs help` for the `list`, `present`, and `remove` commands. One focused card is displayed at a time, with card and content page counters. Tables use a column/row record view; long headers and cells continue across pages without clipping. The bitmap font displays unsupported Unicode and control characters as `\u{HEX}` code-point escapes. Titles and source identifiers may be shortened in the header. Cards are stored in `.space/presentations.json` in this space (one bridge writer). The current limits are 64 worker nodes, up to 128 visible stations including retained/departing workers, and 16 cards per root. Every supported station gets a complete label; long titles wrap instead of being hidden or shortened. Stable cells around the observer separate labels; large workspaces require looking around and up. Labels remain world-space and can be physically occluded, rather than rendering through opaque objects. Upstream discovery limits are indicated separately. Updated spectators accept old frames, defaulting missing cube rotation to identity.

Button actions are processed independently of controller poses and render availability; losing a controller's tracked pose must not discard X presses or releases. Real session focus loss still cancels recording. Headset validation is needed to distinguish missing tracking from Quest runtime/profile changes that deactivate the button actions themselves.

Worker titles, statuses and published cards also appear in the spectator/OBS view: publish only content appropriate for that audience. Worker access to the coordinator's memory is currently restricted by instructions, **not** by a security sandbox. Permissions are never automatically approved; pending child permissions or forms still need to be handled in OpenCode. OpenCode's active status covers foreground session drains and is not proof that all background work everywhere has stopped.

After reviewing a terminal worker result, the coordinator can use `node scripts/workspace.mjs acknowledge --session ROOT --worker CHILD` to clear its visual presence without deleting or archiving the session. `restore` brings it back; clearing a station sends its cubes back toward the coordinator. New activity or a changed outcome also makes it visible again. Active or waiting workers cannot be acknowledged, and visible children keep their ancestors visible. Visibility decisions persist in `.space/worker-visibility.json`.

### Swarm conversation signals (prototype)

Join a space to give the swarm an observable conversational state:

| Appearance | Meaning |
| --- | --- |
| Tight violet circulation, rotating broken halo | The space's agent is working, or input is queued |
| Open, calm mint swarm with a steady complete halo | The agent is idle and nothing is queued |
| Amber, gently pulsing broken halo | An approval or question needs an answer |
| Dim fragmented halo / red fragmented halo | Status unknown / the server is unreachable |

The ready halo persists; you don't need to catch an end-of-turn flash. An active violet arc represents observed work in the space, **not** all delegated/background work. This is status telemetry, not a visualization of hidden reasoning. Outside a space the swarm keeps its original flocking behavior. Controller attraction still works. While in a space, the text console sits lower so it doesn't obscure the swarm.

### Voice

In a space, squeeze the **left grip** to toggle live audio to the other participants and spectator without addressing the agent. The panel shows **MIC LIVE** while the microphone is open. Squeeze again to mute. Live audio is off by default and stops on focus loss, taking the headset off, disconnection, departure or microphone failure; toggle it again to resume. Live capture requests the Android voice-communication preset, but echo cancellation is not guaranteed: use headphones or lower the headset volume if there is feedback. The live-microphone toggle is currently available on Quest, not desktop XR.

Hold **X** to talk to the agent: the panel shows **MIC AGENT** with the words recognized so far, and a **REC** timer above it counts up. Release **X** to send; the server finishes the transcript and queues it like typed input, where **B** can still cancel it. Releasing X returns to live audio if it was enabled; otherwise the microphone is off, and the panel says **Mic off**. Tomori asks for microphone access when it starts, so that the request does not interrupt your first recording; if you dismiss it, the first use of the microphone asks again. If it was denied, allow the microphone for Tomori in the headset's Settings.

If anything stops the words reaching the agent intact, such as lost audio, a dropped connection, nothing being heard or a full queue, the panel shows the error and nothing is sent. Losing focus, as when the system menu opens, cancels a recording that is still held; release **X** before holding it again.

The agent's replies are spoken as they stream in, and everyone in the space hears them and each other's push-to-talk audio through the headset or desktop speakers. You never hear your own recording back. While **X** is held, nothing plays, so that the speakers cannot be transcribed as your words; the reply carries on meanwhile and resumes where it has got to when you release (see [design/speech.md](design/speech.md) and [design/live-audio.md](design/live-audio.md)).

Replies are spoken in full. Any participant can switch a space to brief speech from the **Y** list, or with `set_speech`: each message then speaks its first paragraphs, up to about 20 seconds, then says "More on the panel." A space chosen brief stays brief across a server restart.

The server keeps the latest two spoken replies, up to 5 minutes of audio. If you join a space, or put your headset back on, having missed any of the latest reply, you hear it from its start, then whatever followed, until you have caught up with live speech; others in the space are unaffected. Taking the headset off counts as away, and turns the live microphone off. The headset tells the server when it is put on or taken off, from its proximity sensor and the OpenXR session state, and the status line counts who is away.

Recognition and synthesis run inside the server through sherpa-onnx, whose libraries `scripts/fetch-sherpa-onnx.sh` downloads and checks, and the build links. Recognition uses a streaming transducer that shows words while you speak, and a punctuation model adds capitals and punctuation to the finished transcript; synthesis uses Kokoro. The models need about 560 MB in `.tools/`. From the repository root:

```sh
bash scripts/fetch-asr-model.sh
bash scripts/fetch-tts-model.sh
cargo run -p tomori-server --bin tomori-server -- --backend opencode --speech-root .
```

Speech runs on the CPU, including on macOS; see [crates/speech/readme.md](crates/speech/readme.md) for CUDA on Linux. Without `--speech-root`, or if the recognition model can't load, every utterance fails with an error that says so. Without the punctuation model, which `scripts/fetch-asr-model.sh` also fetches, transcripts are lowercase and unpunctuated, and the server says "Transcript punctuation: unavailable" when it starts. Replies are spoken with Kokoro, which needs another 700 MB; without it the server says "Speech synthesis: off" or "unavailable" when it starts, and replies are only shown.

## PC spectator and live audio (prototype)

The spectator joins the same server and space as the headset and renders a read-only view from a participant's relayed head, controller and hand presence. It renders the shared worker state and simulates its own swarm from the space's activity; it is not a pixel-perfect Quest cast. The headset's HUD is not shown. No OBS or YouTube stream starts automatically.

Both tracked controller grip poses are rendered using the bundled, MIT-licensed WebXR Quest Touch Plus models (see `crates/client-quest/assets/controllers/`). If the Quest runtime exposes `XR_FB_render_model`, the headset attempts to load its models on a background thread and falls back to the bundled meshes if loading fails. The PC spectator currently uses the bundled meshes rather than copying runtime assets. Controller textures are supported for embedded PNG/JPEG and Basis Universal KTX2; button animation is not yet implemented. When `XR_EXT_hand_tracking` is available and hand tracking is enabled on the Quest, tracked hands are shown as a simple joint-and-bone skeleton rather than a skinned hand mesh. Hand gestures do not replace the existing controller bindings. The server relays controller poses and tracked joint positions to the other space participants.

```sh
cargo run -p tomori-quest -- discover --server 127.0.0.1:8765
cargo run -p tomori-quest -- --spectator --server 127.0.0.1:8765 --space SPACE_ID
```

The spectator requests a **1920×1080 physical-pixel** content area at startup, with a 1080p minimum and no window decorations, to match a 1080p OBS canvas. Press **Esc** to close it. Resizing larger changes the render resolution. On a 1080-pixel-high display the desktop panel can cover part of the borderless window; the compositor may also constrain sizes on smaller or scaled displays. Check the actual size in the startup log and OBS preview.

In OBS, add the spectator window as a window/screen capture. The spectator plays participant audio and agent speech through the PC's default audio output; capture that output separately if you want audio. Avoid capturing the same sound twice, and preview audio/video synchronization before going live.

### Experimental direct PipeWire video (Linux)

Instead of capturing a window, run `cargo run -p tomori-quest -- --spectator-pipewire --server 127.0.0.1:8765 --space SPACE_ID`. It publishes `Tomori-Spectator` (`tomori-spectator`) **without creating a window or display surface**. Press **Ctrl-C** to stop it. For an optional window showing the same direct output, use `cargo run -p tomori-quest -- --spectator-pipewire-preview --server 127.0.0.1:8765 --space SPACE_ID` instead; Escape closes that window and stops its output. Run only one direct-output spectator at a time.

Both modes render to a fixed **1920×1080 RGBA** texture at up to 30 fps. The optional preview window requests 1920×1080; the `--spectator` window-capture fallback still renders at its actual window size. The Quest reports presence through the server. Audio plays through the spectator's desktop output, or the PipeWire output named with `--audio-output NAME` (see `pactl list short sinks`); the PipeWire node carries video only. Any local client with PipeWire access may be able to view it.

For the direct virtual camera and microphone used to share this view in a Slack Huddle without OBS, see [the Linux setup notes](docs/slack-huddle.md).

This prototype requires `gst-launch-1.0`, GStreamer's `fdsrc` and `rawvideoparse` elements, and the PipeWire GStreamer plugin. It performs an asynchronous GPU-to-CPU readback and copies through a GStreamer subprocess; it is **not zero-copy**. Readback and the outgoing queue are bounded, so frames can be dropped under load. A slow or absent consumer may briefly see older queued frames when it reconnects. If GStreamer fails, the headless process exits with an error; in preview mode the window stays open but the video source stops. Restart the spectator to retry. Each uncompressed 1080p frame is about 7.9 MiB. A sustained 30 fps is not guaranteed; check performance and latency in the real preview.

The built-in OBS PipeWire screen capture is not the same as a generic PipeWire video input. The [ioquatix fork of `obs-pwvideo`](https://github.com/ioquatix/obs-pwvideo/tree/discover-video-source-nodes) adds an opt-in **Show video devices and other sources** checkbox to the PipeWire Video source; enable it to select **Tomori-Spectator**. The change is under review in [upstream PR #8](https://github.com/hoshinolina/obs-pwvideo/pull/8). To build and install the fork as a Pacman-managed package on Arch:

```sh
cd packaging/obs-pwvideo-ioquatix-git
makepkg -si
```

The fork package conflicts with and replaces the standard `obs-pwvideo` package; don't install both. Review the PKGBUILD before building. OBS must be restarted to load the installed plugin. Rebuild this local VCS package to update the fork; it is not itself an AUR package.

Alternatively, the [AUR `obs-pwvideo` package](https://aur.archlinux.org/packages/obs-pwvideo) works with this stream **when the OBS source already has its target saved** as `tomori-spectator`. The plugin does not discover `Video/Source` nodes in its picker, although it can connect to a saved target of that class. Avoid changing the source's target to “No autoconnect”, which clears it. For a new source, either manually link the PipeWire ports (`pw-link` or a graph editor), or, with OBS closed and the scene collection backed up, set the source's `settings.target` to `tomori-spectator` in its scene collection JSON. Check that the `id` is `pipewire-video-source` and preserve all other settings. An isolated libobs test with the unmodified AUR package and a saved target received frames from the earlier 1080p spectator; check the real OBS preview too.

Alternatively, the supplied compatibility patch makes the spectator selectable in the plugin's picker. To build that patched plugin in a new directory (requires OBS, PipeWire and DRM development headers, CMake and Ninja):

```sh
git clone https://github.com/hoshinolina/obs-pwvideo.git /tmp/opencode/obs-pwvideo-build
git -C /tmp/opencode/obs-pwvideo-build checkout --detach ba64bf322ad450f8fe8c43236878e83669fb677d
git -C /tmp/opencode/obs-pwvideo-build apply "$PWD/patches/obs-pwvideo-spectator.patch"
cmake -S /tmp/opencode/obs-pwvideo-build -B /tmp/opencode/obs-pwvideo-build/build -G Ninja -DCMAKE_BUILD_TYPE=RelWithDebInfo
cmake --build /tmp/opencode/obs-pwvideo-build/build
```

Run these commands from the project directory; the clone directory must not already exist. The plugin is **not installed automatically**. When ready to change OBS, follow [the plugin's installation instructions](https://github.com/hoshinolina/obs-pwvideo#how-to-build-from-source), using the patched build and its `data` directory. Check for an existing installation before copying anything. Do not install both patched and AUR versions at once. OBS must be restarted to load a different plugin; do not restart during a stream. Verify the live preview, performance and audio/video sync before going live. Keep `--spectator` and window capture as the fallback.

### Legacy PC bridge audio (not used by server clients)

The following feeds and helper belong to the older PC bridge. The current server-connected headset and spectator do not publish audio to them; use the spectator's desktop audio output instead. These notes are retained only for the legacy bridge.

For separate legacy OBS volume controls, use `/spectator/microphone.wav` and `/spectator/agent.wav` as two Media Sources instead of the mixed `audio.wav`. All three are live 16 kHz mono streams on the same loopback bridge; don't add both the mixed stream and the individual streams or the sound will be doubled.

### Experimental PipeWire/PulseAudio inputs (Linux)

For the legacy bridge only, as an alternative to its OBS Media Sources, keep that bridge running and start the audio helper in a separate terminal:

```sh
node bridge/pipewire-audio.mjs
```

It exposes **Tomori-Quest-Microphone** and **Tomori-Assistant-Voice** as two audio inputs. In OBS, add an **Audio Input Capture (PulseAudio)** source for each and choose the matching device. Start the helper before adding or selecting the OBS sources; leave the existing OBS scene unchanged until you are ready to test. Do not simultaneously add the WAV Media Sources for the same audio.

The helper subscribes to the bridge's existing local HTTP/WAV feeds and publishes them through PipeWire's PulseAudio compatibility layer; the headset-to-PC transport is unchanged. It creates two dedicated silent output sinks and two remapped inputs in the current user session, without deliberately changing the default devices or sending audio to the speakers. The internal sinks and their monitor sources can also appear in device lists. Only one helper can run at a time. It requires `pactl`, `gst-launch-1.0`, GStreamer's `rawaudioparse`, `queue` and `pulsesink` elements, and a PulseAudio-compatible server supporting `module-null-sink` and `module-remap-source`.

When the bridge disconnects, the helper emits silence and retries; it can stay running across a bridge restart. **The OBS PulseAudio input did not reconnect automatically after the helper itself was stopped and restarted** in an isolated test. In that case, mute the affected OBS sources and reselect their devices or recreate the sources when safe. Ctrl-C or SIGTERM stops the helper and removes the virtual devices; an uncatchable termination or Pulse server failure can leave modules behind. If the helper reports an existing device, inspect `pactl list short modules` and unload only the matching stale `module-null-sink` and `module-remap-source` IDs after confirming they belong to this helper.

The original audio remains 16 kHz mono even if PipeWire exposes the remapped inputs at 48 kHz. Small buffers mean late audio may be dropped, and the pipeline can retain a short tail after muting; verify the headset's `MIC OFF` state and audio/video sync before broadcasting.

## Build and sideload (Linux)

Enable Quest developer mode, connect a USB data cable, and accept **Allow USB debugging** inside the headset. `adb devices -l` must show `device`, not `unauthorized`. Keep your boundary enabled and stay seated or stationary. Wake the Touch controllers before launching: Quest may otherwise show a **controllers required** dialog and block the application from starting.

Install a **user-local rustup** toolchain (Rust 1.90+) and Android tools:

```sh
rustup target add aarch64-linux-android
cargo install cargo-apk --version 0.10.0 --locked
```

With Android SDK command-line tools installed, install these SDK components (review/accept Google's licenses when prompted):

```sh
sdkmanager 'platform-tools' 'platforms;android-32' 'build-tools;35.0.0' 'ndk;29.0.14206865'
export ANDROID_HOME="$HOME/Android/Sdk"
export ANDROID_NDK_HOME="$ANDROID_HOME/ndk/29.0.14206865"
bash scripts/android.sh run
```

The script downloads the official Khronos **1.1.65** OpenXR loader AAR from Maven Central, checks its published SHA-256, and packages the arm64 loader in the APK. The Quest APK does not contain a transcription model. `cargo-apk` creates a local debug signing key if needed. Build output: `target/debug/apk/tomori.apk` (package `io.github.ioquatix.tomori`). Launch from **Library → Unknown Sources → Tomori** if not auto-started.

```sh
bash scripts/android.sh devices
bash scripts/android.sh logs
bash scripts/android.sh build  # build without deploying
```

You can save machine-local exports in `.android-env` (gitignored). The SDK, NDK, JDK (`java`/`keytool`), `cargo-apk`, and ADB must be available. Debug builds use optimization level 1; the example is for local sideloading, not store submission. This machine uses isolated tools in the ignored `.tools/` directory, referenced by `.android-env`; system Rust is not modified.

## Desktop window

```sh
cargo run -p tomori-quest
```

Runs the same simulation and wgpu renderer in a resizable desktop window; **no headset or OpenXR runtime needed**. Uses Vulkan on Linux (Wayland or X11).

- **Hold left mouse:** attract agents to the cursor on a plane through the swarm.
- **Right-drag:** orbit the camera; **mouse wheel:** zoom.
- **R:** reset agents; **Space:** pause/resume; **Esc:** quit.

That window doesn't connect to a server. To take part in a server's spaces from a desktop window instead, as on the headset, use `--desktop` (see [design/desktop.md](design/desktop.md)):

```sh
cargo run -p tomori-quest -- --desktop [--server HOST:PORT] [--name NAME] [--token TOKEN]
```

The mouse points as the right controller does: its left button chooses what it points at, and dragging with the right button looks around. **Enter** is A, **Backspace** is B (hold it for a second to leave a space), hold **Space** to talk to the agent (X), **Tab** is Y, the arrow keys are the left stick, **\[** and **]** the right stick left and right, and **Page Up**/**Page Down** the right stick up and down. **M** opens the hand menu, **K** knocks on the space you watch, **+** and **-** admit or decline the first knock waiting, **P** pairs, **V** mutes spoken replies and **R** replays the latest. `tomori --help` lists them too. Desktop windows draw with 4× multisample anti-aliasing where the GPU supports it.

To use a server without a headset or a GPU, show the headset's panel in the terminal:

```sh
cargo run -p tomori-quest -- --console [--server HOST:PORT] [--name NAME]
```

Type the controller buttons as commands, such as `a`, `b`, `y`, `u` and `d` for the left stick, and `<` `>` for the right stick. A terminal can't tell when a key is released, so `x` starts talking to the agent through the computer's default microphone and `x` again sends it. `--server` (or `TOMORI_SERVER`) and `--name` also apply to `--xr`, where **X** is held as on the headset. On Linux, building the desktop client needs the ALSA development files (`libasound2-dev` or `alsa-lib-devel`) for the microphone.

## Tests / desktop OpenXR

```sh
bash scripts/fetch-sherpa-onnx.sh        # once: the speech libraries the server links
bash scripts/fetch-asr-model.sh whisper tiny.en && bash scripts/fetch-tts-model.sh   # optional: the speech models the speech tests run
cargo test --workspace                   # simulation, projection, rendering, server and client tests
cargo run -p tomori-quest -- --xr        # use an active desktop OpenXR Vulkan runtime
npm test --prefix bridge          # session routing, questions and permissions
```

The rendering tests draw offscreen with Vulkan: on a GPU when there is one, and otherwise on a software driver such as Mesa's lavapipe (`mesa-vulkan-drivers` on Debian and Ubuntu, `vulkan-swrast` on Arch). With no Vulkan adapter at all they are skipped, each saying so; set `TOMORI_REQUIRE_GPU=1` to fail them instead, as CI does. The tests in each test binary share one Vulkan device, created once and never dropped, because Vulkan loaders before 1.4.346 can crash when one thread creates a device while another uses one. To render on lavapipe, as CI does, on a computer with a GPU, name its driver file in `VK_DRIVER_FILES`, such as `VK_DRIVER_FILES=/usr/share/vulkan/icd.d/lvp_icd.x86_64.json`. Every `.wgsl` shader is also parsed and validated with naga, with or without an adapter.

The speech tests that run the real recognition, punctuation, voice activity and synthesis models are skipped, each saying so, when the models aren't installed; set `TOMORI_REQUIRE_MODELS=1` to fail them instead, as CI does. They use Whisper `tiny.en` rather than the server's default, `small.en`; see [crates/speech/readme.md](crates/speech/readme.md#tests).

`cargo bake test` runs the workspace tests too, fetching the speech libraries first. On every pull request, GitHub Actions checks formatting and Clippy, runs the tests, including the rendering tests on lavapipe and the speech tests with their models, on current stable Rust and on the minimum supported version (the workspace's `rust-version`, 1.90), and builds the client core and the headset APK for `aarch64-linux-android`. The main branch requires these, through the Test workflow's `test-result` job, and the Publish workflow's `check` job. `cargo bake test:coverage` checks the 100% source-region coverage that Base projects require; the Coverage workflow runs it on every pull request, but it is not required yet, because coverage is still well short of it ([#55](https://github.com/ioquatix/tomori/issues/55)).

For the PC streaming-recognition test (model + public sample required):

```sh
bash scripts/setup-pc-transcription.sh
mkdir -p target/downloads
curl -fL https://raw.githubusercontent.com/ggml-org/whisper.cpp/master/samples/jfk.wav -o target/downloads/jfk.wav
(cd bridge && node --test asr.test.mjs)
```

It checks partial results, finalization, out-of-order rejection, and a recording longer than a minute.

A USB-connected Quest does **not** provide a Linux desktop OpenXR runtime: build the Android APK for standalone Quest use. Desktop execution requires its own OpenXR loader/runtime (for example Monado).

## Layout / interop

- `crates/client-quest/src/desktop.rs`: desktop window, orbit camera, mouse attraction.
- `crates/client-quest/src/xr.rs`: Android/desktop OpenXR lifecycle, predicted views, controller actions: the device calls only.
- `crates/client-quest/src/scene.rs`: the frame step the headset and the desktop window share, from plain input; `scene/headset.rs` and `scene/desktop.rs` hold each platform's own logic, and `scene_graphics.rs` draws a frame.
- `crates/client-quest/src/vulkan.rs`: XR-created Vulkan instance/device imported into wgpu; borrowed two-layer XR swapchain images, one layer per eye.
- `crates/client-quest/src/graphics.rs`, `crates/client-quest/src/swarm.wgsl`: instanced cube rendering with per-eye depth.
- `crates/client-quest/src/swarm.rs`: deterministic CPU flocking, separation/alignment/cohesion.
- `bridge/asr.mjs`, `bridge/asr_worker.py`: GPU/CPU Whisper rolling transcription.
- `crates/client-quest/src/caption.rs`, `crates/client-quest/src/caption.wgsl`: head-relative, stereo text panel.
- `crates/client-quest/src/conversation.rs`: server list, space list and space panel, driven by server events.
- `crates/client-quest/src/connection.rs`: the server connection and Bonjour browsing.
- `bridge/`: official OpenCode V2 client adapter, no longer used by the headset.

The Vulkan device uses the runtime-selected physical GPU and enables the full extension/feature set required by wgpu. Swapchain images remain runtime-owned; wgpu has no permission to destroy them. Each frame waits for submitted GPU work before releasing the XR image, leaving it in `COLOR_ATTACHMENT_OPTIMAL`. This simple conservative synchronization favors correctness over maximum throughput. Tracking loss and non-rendering frames submit no layer; sessions handle ready/stopping/exit events. The scene initially faces the headset; the app uses `LOCAL_FLOOR` when available so the Quest's system Meta-button recenter moves the scene, falling back to `LOCAL` with an approximate 1.6 m floor offset. Reference space changes take effect at the runtime's specified frame time. Recentering doesn't reset the agents, conversation, or interaction state. This is opaque VR, not passthrough; optional tracked-hand skeletons are described above.

## Crates

The workspace's crates are named `tomori-` followed by their role, in folders under `crates/` without the prefix. They share one version and one [releases.md](releases.md), and are released together. These are published to crates.io, though none has been published yet:

- [`tomori-protocol`](crates/protocol/readme.md): the wire protocol shared by clients and the server.
- [`tomori-client`](crates/client/readme.md): a client's connection, reconnection, microphone audio and playback.
- [`tomori-discovery`](crates/discovery/readme.md): Bonjour advertising and browsing of servers.
- [`tomori-security`](crates/security/readme.md): tokens, pairing and certificate pinning.
- [`tomori-backend`](crates/backend/readme.md) and [`tomori-backend-opencode`](crates/backend-opencode/readme.md): the agent backend interface, and its OpenCode implementation.
- [`tomori-speech`](crates/speech/readme.md): speech recognition and synthesis with sherpa-onnx.
- [`tomori-server`](crates/server/readme.md): the server, installed with `cargo install tomori-server`, and the library another project builds a server of its own on, registering its own backends and environments through `Server::builder()`. [examples/server](examples/server/readme.md) is an example.

`tomori-quest`, the headset and desktop application in `crates/client-quest`, is released as an APK instead, and [`tomori-environment-sky`](crates/environment-sky/readme.md) stays private until environments have a shared interface. [design/crates.md](design/crates.md) gives the reasons, and [docs/naming.md](docs/naming.md) records that the names were free on crates.io.

## Third-Party Components

Original project code is released under the MIT License; see [license.md](license.md). Third-party components retain their own licenses:

- Bundled controller assets: [MIT](crates/client-quest/assets/controllers/LICENSE.md).
- Basis Universal notices and test fixtures: [Apache-2.0](crates/client-quest/assets/licenses/basisu-LICENSE.txt), with fixture provenance in [crates/client-quest/tests/fixtures/README.md](crates/client-quest/tests/fixtures/README.md).
- The optional OBS plugin compatibility patch follows the upstream plugin's GPL-2.0-or-later licensing; its packaging metadata is under `packaging/`.
- Speech models and native dependencies are downloaded separately, not bundled in this repository. Their licensing requirements are noted in the relevant setup sections above.

## Releasing

Prepare a release with `cargo bake cargo:version:patch` (or `minor`, `major`, or `bump --version X.Y.Z`), then run `cargo bake cargo:release` and open a pull request. After review and merge, GitHub Actions publishes the release when the configured `crates-io` environment approves it. Follow the shared [Releasing skill](https://github.com/base-rs/base-project/blob/main/context/releasing.md) for the standard process.

No Tomori crate has been published yet. A release publishes the crates listed under [Crates](#crates), which `cargo bake cargo:packages` lists, and `cargo publish --workspace --dry-run --locked` checks that they package and build.

## Releases

<!-- bake-readme:releases:start -->

See [releases.md](releases.md) for the full release history.

<!-- bake-readme:releases:end -->

## See Also

- [tomori](https://github.com/ioquatix/tomori) — A conversational spatial workspace for people and agents, built with OpenXR and wgpu<!-- bake-readme:package -->

## Contributing

Please open an issue or pull request on [GitHub](https://github.com/ioquatix/tomori).

### Agent Context

Run `cargo bake agent:context:install` to install shared context and skills. Use `cargo bake agent:context:list` and `cargo bake agent:context:show` to discover relevant guides, follow `agents.md` if present, and apply skills under `.agents/skills/`. The installer preserves repository-owned `agents.md`; it does not create or regenerate that file.

Tomori's published crates provide context of their own for projects that depend on them, in [crates/protocol/context/](crates/protocol/context/), [crates/client/context/](crates/client/context/) and [crates/server/context/](crates/server/context/).
