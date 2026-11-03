# Releases

## Unreleased

- While you hold **X** to talk, a small card rises from the left controller, where X is, instead of a timer floating above the panel: it shows how long you have been talking against the server's limit, with a bar, the last words heard, and "Release X to send". In the last 30 seconds before the limit it turns amber and says how long is left, and red in the last 10. When you let go it says "Sending…", then where your words went, such as "Queued, 2nd", or "Not sent" with why until you press **X** again or dismiss it with **B**. While the live microphone is on, a small `LIVE` badge stands on the left controller. Without the controller tracked, the card follows your left wrist, or stands low in your view.

- `tomori-server` tells clients how long a voice message may be, in its `Welcome`, as `utterance_limit` in seconds: 5 minutes with `--speech-backend offline`, and none with the streaming backend.

- The panel and the new controller cards are drawn in the world with the rest of the scene, translucent, so that a hand or controller in front of them covers them. The cards' text can show accented letters, Greek, box drawing and a few symbols, rather than `?`.

- The headset draws with 4× multisample anti-aliasing, as desktop windows do, so edges are smooth instead of jagged. The samples stay in the GPU's tile memory and are resolved as each eye is drawn. Start it with the intent extra `tomori.msaa` set to `off` to compare, or if it costs frames.

- Point at a worker and ask about it, as an early prototype of shared attention (#81). In a space, the right controller's ray, or the desktop's mouse, highlights the worker it points at. The headset draws the ray in a space as in arrival: it reaches the worker it highlights, and is short and plain when it meets none. Point at one as you press **X**, or while you talk, and your words reach the agent with a line naming that worker, its session and its state, such as `[Pointing at worker "Review commit 2657a52" (opencode session ses_…, running)]`; pointing only before you press **X** doesn't count. The server describes the worker from its own state, never from what the headset says. The trigger still attracts the swarm. In `--console`, `point` lists the workers and `point N` points at one. Older servers ignore the pointing, and older clients send none.

- Tomori now gives the agent instructions of its own in each space, so that it answers briefly in plain speech and hands non-trivial work to sub-agents instead of doing it inline. They come in three layers: Tomori's own, the environment's (none for Sky yet) and the space's, which an owner can give when creating a space and change later. With OpenCode they are set as the session's instruction entries, never in your configuration or `AGENTS.md`, and are removed when the server forgets the space. Everyone in a space can read them in the hand menu's new **Agent** section, where an owner can also clear the space's own. In `--console`, `i` shows them, and `instruct TEXT` gives the space you are in, or the next one you create, its own instructions. If OpenCode refuses them, the space still works and says why.

- As the user asked in November 2026, the agent keeps the workspace clean of finished sub-agents: Tomori's instructions ask it to remove a sub-agent once it has finished and its findings are reported, unless someone asks to keep it.

- In a space, **B** never denies an approval, cancels your words or leaves by accident: it closes what is open or dismisses a problem, and you hold it for a second to leave. *Deny* is one of the approval's choices on the right stick, confirmed with **A**. Select any waiting entry in the queue strip with the right stick up and down, and press **A** to cancel it; **A** again within 3 seconds undoes it.

- Errors get their own line on the panel, marked **!** with how long ago they happened, and stay until dismissed or replaced instead of hiding behind "Sending…" or vanishing on the next action. The queue is always shown in its own strip.

- **Y** opens a list of the agent's modes with their descriptions, and the space's brief or full speech, and **A** chooses one directly instead of cycling through them.

- Hold **X** while a question is shown to answer it by voice, with an option's label or number; the words go to the question, not the queue.

- The panel stands in the world in front of you instead of following your head, and glides back in front of you only when you turn or walk away for more than a second.

- A space keeps its spoken replies while nobody is listening, and plays the latest one from its start when you join or put your headset back on, instead of joining mid-sentence; then whatever followed, until you have caught up with live speech. It keeps the latest two replies and at most 5 minutes of audio. Others in the space are unaffected, and nobody hears a reply twice at once. Older clients and the spectator stay with live speech as before.

- Click the right stick in a space to hear the latest reply again from its start, to you only, and again to stop it; a new reply stops it and plays live. Click the left stick to mute spoken replies for you only. In `--console`, `replay` and `mute` do the same.

- The headset tells the server when it is taken off or put back on, from its proximity sensor where the runtime reports it and from the OpenXR session state, and turns the live microphone off when it is taken off. Everyone in the space sees who is away, as "2 here, 1 away".

- Replies are spoken in full by default. A space can be switched to brief speech from the **Y** list or with `set_speech`: each message then speaks whole paragraphs up to about 20 seconds, and a reply cut short says "More on the panel." once. A space chosen brief stays brief across a restart; every other space, including one kept before, speaks in full. A closed space keeps no speech.

- The headset shows which servers and spaces need you. A server whose spaces wait for an approval or answer glows amber, one with a problem nobody saw glows red, and one with a reply nobody heard glows a soft white, and its label says what is waiting, such as "1 needs you, 2 replies". A small jump object above it connects and joins the space that needs you most in one trigger pull: the highest level, then the one waiting longest. If someone dealt with it meanwhile, you stay in the server's lobby and are told so. In the lobby, spaces keep their places, are marked and labelled with what they need, and the selection starts on the most urgent one. The remembered server is now connected a moment, about one and a half seconds, after it is found, and not at all if another server found by then needs you and it doesn't. Attention is for anyone: whoever joins a space or answers its request clears it for everyone, and being in a space when something happens counts as having seen it. The lobby listing carries each space's attention and how long it has waited, in optional fields. `tomori-server` advertises how many spaces need input, have a problem and have a reply in its Bonjour TXT record, as `waiting`, `problems` and `replies`; `--no-advertise-attention` leaves just those out.

- The crates need Rust 1.90 or later rather than 1.93: wgpu-hal's dependency on ordered-float 5.5 needs 1.90, and nothing in Tomori itself needs more than 1.88. GitHub Actions now builds and tests every pull request on stable Rust and on 1.90, and builds the headset APK.

- `tomori-server` keeps serving everyone else, and keeps releasing silent connections, while one client sends faster than the server can apply its input, instead of stalling until that client pauses. The counts it advertises also include a client that connects just as it starts.

- `tomori-server` no longer has a Claude Code backend, nor its `--claude-permission-mode` option: use `--backend opencode`, which can use Claude models too and restores a space's conversation and pending approvals when the server restarts.

- `tomori-server` keeps its spaces across restarts, in `spaces.json` in its state directory: each space is listed again as soon as the server starts and bound to its agent session again, so the headset returns to the same space and collaborators stay admitted. A space whose session cannot be resumed is listed as closed with the reason. Input still queued when the server stops is dropped.

- A visitor can knock to take part in the space they watch, and its participants let them in or turn them away; someone let in becomes a collaborator in that space.

- Turn your left palm, or left controller, towards your face to show the hand menu above your wrist: its People & security section shows who is here, how many visitors are watching, knocks waiting, pairing and notices, and flashes when someone knocks.

- Pair the headset with a server to make it an owner, without copying a token: choose the gold key beside the connected server, and confirm the 6-digit code it shows by typing it on the server's console, or on the owners' status page. Pairing also restores trust in a server whose certificate changed. In `--console`, `p` pairs.

- `tomori-server` admits by token: holders of its tokens are owners, who create spaces, or collaborators, and anyone else may watch and listen as a visitor, unless `--no-visitors`. Make the first owner token with `tomori-server auth mint --owner --name NAME`, and give it to the headset with `TOMORI_TOKEN` (the `tomori.token` intent extra) or to the desktop with `--token`; `auth list` and `auth revoke` manage them. `--auth open` admits everyone as an owner, as before.

- `tomori-server` serves only TLS when it admits by token, with a self-signed certificate kept in its state directory (`~/.local/state/tomori` by default) and advertised by fingerprint; clients pin it the first time they connect and send their token to no other certificate. Owners sign in with a token at `https://<server>:8765/` to see the server's spaces, who is in them and its tokens, and to revoke one.

- The headset offers **New space** only to owners, and a visitor watches a space without talking, live audio or presence.

- Other participants can no longer take over your place in a space: the server no longer sends anyone's participant identity to other clients, and names participants in a space and its queue by their number instead. The headset chooses a new identity once, since its old one was shared. This changes the protocol to version 2, so update the headset and the server together.

- `tomori-server` survives an OpenCode service restart or a dropped event stream instead of closing every space and refusing new ones until it is restarted. It finds the service again, including a new port and password from `service.json`, and reconnects with a backoff of up to 10 seconds. Requests made while OpenCode is down fail within seconds, and creating a space works again as soon as it returns. Open spaces show that the connection was lost, hold queued input, and refuse decisions and mode changes with a message until each is read again from its session, so a permission or question raised during the gap still appears. Shutting down no longer waits for requests held while OpenCode is unreachable or for one OpenCode has not answered. A reply that was streaming when OpenCode's event stream dropped keeps the text already shown, and is shown in full once it ends. A space whose session was deleted, or that cannot be read again, closes and says why. Nothing said during the gap is spoken, and a worker deleted during the gap stays listed until the space is attached again.

- A space that closes now always shows why, rather than sometimes reporting only that its event channel closed.

- `--speech-backend offline` transcribes voice messages of up to 5 minutes instead of 30 seconds. A message longer than Whisper's 30 second window is split at its pauses, heard by the Silero voice activity detector, or by loudness when it is not installed, and the parts are transcribed in order by the same model, so a long message is as accurate as a short one; what Whisper writes for a long pause is left out: a note such as "\[BLANK_AUDIO]" always, and a word or two such as "you" when the detector is installed and hears no speech there. A message longer than 5 minutes fails as soon as it passes the limit, saying so, and nothing is sent. While one participant's long message is transcribed, another's waits for at most one 28 second part of it. Run `bash scripts/fetch-asr-model.sh whisper` again to fetch the detector.

- The headset logs much less to logcat, so connections, joins, utterances and errors stay in its buffer for many minutes. It logs its first submitted frame and then a frame count and rate once a minute instead of every 720 frames. Swarm interaction modes, pending reference-space changes and controller pose changes are logged only at debug. Repeated recenters are logged at info at most once a minute, and live microphone overruns at most once every 30 seconds, with the overruns counted in between. Speaker underruns on the desktop are logged only at debug.

- Offer only top-level OpenCode sessions when attaching a space, not subagents' child sessions or archived sessions.

- The server, client and OpenCode backend use the Native scheduler instead of the Direct scheduler. An idle server no longer wakes every 2 ms to retry its sockets, and accepting connections can no longer stall on macOS once the Direct blocking pool stops waking its workers.

- Track OpenCode child lifecycle events so deleted workers animate their return without mistaking completion or partial lists for deletion.

- Document the direct 1080p virtual camera and dedicated audio input used to share a Tomori space in a Slack Huddle without OBS.

- Toggle the Quest microphone for live conversation with the left grip; hold X to address the agent and release it to return to the previous live setting. Live capture stops on focus loss or disconnection. Echo cancellation is not guaranteed.

- The spectator renders an unobstructed view without the headset HUD, animates the swarm from the space's activity, and publishes 1080p PipeWire video instead of 4K.

- Release Tomori under the MIT License only, replacing the MIT or Apache-2.0 dual license.

- The headset joins spaces on a Tomori server instead of chatting through the PC bridge: it lists servers found with Bonjour and the USB connection, reconnects to the last server when it finds it again, then lists the server's spaces. It reconnects and rejoins after a dropped connection, says when a space has ended, as after the server restarts, and answers approvals with the right stick and **A**.

- Choose the server and display name with the intent extras `tomori.server` and `tomori.name`, `TOMORI_SERVER=host:port bash scripts/android.sh run` (with `TOMORI_SERVER_PORT` for the port forwarded over USB), or `--server` and `--name` on the desktop.

- Add `--console`, which shows the headset's panel in the terminal for using a server without a headset or a GPU.

- Remove the PC bridge's voice path, spoken replies, voice preview, replay, result cards and spectator publishing from the headset for now.

- Hold **X** in a space to talk to its agent through the server: the headset sends push-to-talk audio over its server connection, shows the words recognized so far and the microphone's state on the panel, and the server transcribes them and queues the text. Lost audio, a dropped connection, nothing heard or a full queue fails visibly instead of sending anything. In `--console`, `x` starts talking through the computer's default microphone and `x` again sends it.

- Speech recognition and synthesis run inside `tomori-server` through sherpa-onnx instead of Python workers, so the server no longer needs Python for speech. Recognition streams: a streaming transducer shows the words recognized so far while you speak, and the transcript is ready about a tenth of a second after you let go. Transcripts are now lowercase, with no punctuation. Fetch the sherpa-onnx libraries, checked against a pinned checksum, with `bash scripts/fetch-sherpa-onnx.sh` before building the server, and the models with `bash scripts/fetch-asr-model.sh` and `bash scripts/fetch-tts-model.sh`, then run `tomori-server --speech-root .`, on macOS or Linux. Recognition that falls more than 5 seconds behind fails the utterance visibly. If recognition or synthesis stops after an internal error, participants are told it is unavailable until the server restarts. `--speech-provider cuda` needs a server built with the `speech-shared` feature. The PC bridge's transcription still uses `scripts/setup-pc-transcription.sh`.

- `tomori-server` keeps serving after transient accept errors, such as running out of file descriptors, instead of silently leaving its port open with nobody accepting. When serving does end it refuses new clients at once, waits at most 5 seconds for open connections, then at most 3 seconds for the speech engines to stop, and exits with a failure status if serving failed. It now logs to standard error, controlled by `RUST_LOG`.

- `tomori-server` speaks each paragraph of a reply once it has ended, rather than sentence by sentence while it is still arriving, and speaks a list once it ends. A paragraph that leads straight into a list or code block without a blank line is spoken once that list or code block has ended. A line that turns into a table header or heading is no longer spoken first as a paragraph.

- Each space is bound to one environment, chosen when it is created and the same for everyone in it. The server offers its environments, today only Sky, and refuses one it doesn't offer. The headset waits on the Minimal grid outside a space, shows the space's environment inside it, and returns to the grid when it leaves; squeezing the right grip no longer switches environments. A space in an environment the headset doesn't know shows the grid and names the environment. `TOMORI_ENVIRONMENT` and **E** now apply only to the desktop window.
