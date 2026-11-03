# Live Audio

This describes the Rust server and the headset client, which uses it. The current Node bridge differs; changes to it are not reviewed against this document.

## Requirement

Participants in a space can talk to each other. Each participant sends one microphone stream, and each audio frame carries a gate:

- `off`: nothing is sent.
- `live`: heard by the other participants, visitors watching the space (see `security.md`) and the spectator feed.
- `agent` (push-to-talk): heard by the other participants, visitors and the spectator feed, and also transcribed for the agent.

Visitors send no audio; the server neither relays nor transcribes audio from a visitor's connection.

Push-to-talk audio is not a separate upload path. Only `agent` audio is transcribed; open-microphone conversation between participants never reaches the agent.

Live audio is explicitly toggled and off by default. The Quest left grip toggles it, and the panel shows when the microphone is live. Holding X temporarily switches to the agent gate; releasing X returns to the previous live setting. Losing headset focus, disconnecting, leaving the space, or a microphone failure stops live capture; it does not resume without another explicit toggle. Platforms without a capture implementation that can stop independently of the render thread refuse live capture rather than leave a microphone open after disconnection.

Captured audio belongs to the connection it was captured for. A client binds each capture to its connection and never sends that capture's audio on a later connection, even after the microphone is rearmed for it; reconnecting does not reopen the microphone. Once a connection has ended, no further audio captured for it is sent. On Quest, live capture checks the connection again after the device opens and does not start it if the connection has ended; a disconnection racing the start itself stops the stream before anything is read.

Push-to-talk ends when the client processes the release, within about a frame of it. On Quest, every sample the device has delivered by then is sent before the utterance ends: a read in progress and what the stream already holds, including a final partial frame. Audio still in the device's own pipeline at that moment, typically a few milliseconds, and up to one read after the release may be included in the utterance. Push-to-talk always captures with the recognition input preset, even when live audio was on. If the audio cannot be read intact, including when the device reports an overrun, the utterance fails visibly rather than being transcribed with a gap. On desktop, the capture library delivers what it has captured until its stream is closed at that boundary.

Each client mixes every incoming participant stream together with the space's agent speech stream into a single output.

What a participant records never plays back to them, through any gate. The server never relays a participant's audio to that participant, and a client also discards any audio attributed to its own participant number, in case a server does. The agent's speech is not the participant's own audio: its reply to a question they asked plays to them like everyone else.

A client that shows who is speaking takes it from the audio its mixer actually plays, measured before local volume, so muting agent speech locally silences it without hiding that the agent is speaking. The server mixes the same sources once for the spectator feed.

Playback and mixing are real time: audio that arrives too late to play is dropped from playback rather than delayed or replayed. This does not apply to transcription: every `agent` frame reaches recognition, in order, and a gap in `agent` audio fails the utterance with a visible error, as `speech.md` requires.

The agent speech stream is real time too. Synthesis runs ahead of playback, but the server sends each segment's audio at the rate it plays, only slightly ahead to absorb network jitter, to whoever is in the space at that moment; a participant who joins mid-segment hears it from that point. On each connection, participant audio never displaces agent speech: agent frames are queued separately and dropped only when they could no longer play in time, which receivers see as a gap in the agent stream's sequence numbers.

Participant playback should not feed back into their microphone stream:

- While a client records push-to-talk, it silences its own playback, fading out over a few milliseconds, so that agent speech and other participants playing through its speakers are not transcribed. Silenced audio is still consumed in real time, so when recording ends playback resumes live rather than replaying what was missed. Silencing is local: the agent speech stream and what other participants hear are unchanged. The client silences playback before opening a microphone for push-to-talk, or before switching an already-live microphone to the agent gate. Audio the output device has already buffered, typically a few tens of milliseconds, can still sound as recording starts; that is accepted for push-to-talk.
- An open (`live`) microphone does not silence playback, so participants can hear each other. The Quest requests Android's voice-communication input preset for live capture, which may provide echo cancellation on some devices but does not guarantee it. Until cancellation is verified on the device or a software canceller is integrated, users should use headphones or lower speaker volume if they hear feedback. Push-to-talk still silences local playback.

## Rationale

The existing audience microphone and push-to-talk recording are separate paths that only reach OBS and the transcription bridge. A shared workspace needs people to hear each other, including when someone addresses the agent. Agreed in the October 2026 design discussion.

A participant hearing their own recording back is disorienting and, through their microphone, feeds back; the user required in November 2026 that what a user records must not play back to that user. Silencing playback during push-to-talk was chosen because the Quest's `VoiceRecognition` capture preset and desktop capture have no echo cancellation, and without it a spoken reply playing while the participant records is transcribed and queued as their input, looping the agent's words back to it.

In November 2026 the user asked to restore the live toggle and explicitly chose to allow it with best-effort echo cancellation and a feedback warning rather than block it until cancellation is verified. This replaces the earlier requirement to block live audio until echo cancellation is available: the Quest requests the communication preset, while the actual availability and effectiveness of cancellation remain unverified and must not be presented as guaranteed.

## Scope

Client audio capture and playback, the microphone gate, server routing and the spectator mixer.

## Validation

- Test that `agent` frames are both forwarded and transcribed, `live` frames are only forwarded, and `off` produces no frames.
- Test that a client's output contains other participants and agent speech but not its own microphone, and that muted agent speech plays silently while still reporting a level.
- Test end to end that a participant who records, through every gate, while the agent answers them receives the agent's reply and no frame of their own; and that a client given audio attributed to its own participant number does not deliver it.
- Test that silenced playback fades out, consumes audio in real time while silenced and resumes live, and inspect that playback is silenced before an agent utterance begins and until it ends, whether or not live capture was already open.
- Test that frames arriving too late for playback are dropped from playback and mixing, while the same `agent` frames still reach recognition.
- Test that a missing `agent` frame fails the utterance visibly instead of producing a partial transcript.
- Test that agent speech is sent at its playback rate rather than as fast as it is synthesized, that a participant joining mid-segment receives the rest of it, and that a backlog of participant audio on a connection does not displace agent frames.
- On a headset and a desktop, confirm that agent speech and other participants playing through the speakers are not transcribed from the local microphone while push-to-talk is held.
- Test, with a scripted capture device, that a disconnect before or while it opens never starts it, that a disconnect during a read delivers nothing further, that a release during a read delivers that read and the device's remaining buffer including a final partial frame, that a failed final read or an overrun fails push-to-talk while an overrun only glitches live audio, and that push-to-talk during live audio captures with the recognition preset. Test that audio bound to an ended connection is not sent on a later, rearmed connection.
- Test live-to-agent-to-live switching and that focus loss, disconnect, leaving the space and capture failure stop live audio. On Quest, test for speaker-to-microphone feedback with two participants before relying on echo cancellation.

Latency targets for participant-to-participant audio are not yet agreed.
