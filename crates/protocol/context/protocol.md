# Using the Tomori Protocol

`tomori-protocol` defines what a Tomori client and server send each other. Use this guide when encoding or decoding the protocol directly, or when changing messages. To connect a client, prefer `tomori-client`, which implements the rules below; see [Tomori Crates](crates.md).

## The connection

A client opens one long-lived connection with `POST` to `CONNECT_PATH` (`/connect`) on port `DEFAULT_PORT` (8765), with the content type `CONTENT_TYPE` (`application/x-tomori-frames`). The request body carries the client's frames and the response body carries the server's, for the life of the connection. A server that admits by token serves only TLS, with a self-signed certificate that clients pin by fingerprint; tokens are presented on the HTTP request, never in a message.

## Frames

Each direction is a sequence of `Frame`s: a kind byte, a little-endian `u32` payload length, then the payload.

- `FrameKind::Control` carries one JSON control message, `ClientMessage` or `ServerMessage`, tagged by a snake-case `type` field, such as `{"type": "join", "space": "…"}`.
- `FrameKind::Audio` carries one `AudioFrame`: a 24-byte header with its gate, source, sequence, sample rate and timestamp, then 16-bit mono PCM samples.

Build frames with `Frame::client`, `Frame::server` and `Frame::audio`, and encode them with `Frame::encode`. Read them with a `FrameDecoder`: `push` the bytes as they arrive, call `next_frame` until it returns `None`, and convert each frame with `to_client_message`, `to_server_message` or `to_audio`. No frame exceeds `MAX_FRAME_BYTES` (1 MiB); the decoder refuses a larger one, and `Frame::try_encode` fails rather than producing one.

```rust
use tomori_protocol::{Frame, FrameDecoder, FrameKind, ServerMessage};

fn receive(decoder: &mut FrameDecoder, bytes: &[u8]) -> Result<Vec<ServerMessage>, tomori_protocol::FrameError> {
    decoder.push(bytes);
    let mut messages = Vec::new();
    while let Some(frame) = decoder.next_frame()? {
        match frame.kind {
            FrameKind::Control => messages.push(frame.to_server_message()?),
            FrameKind::Audio => {
                let _audio = frame.to_audio()?;
            }
        }
    }
    Ok(messages)
}
```

## Conversation

1. The client sends `ClientMessage::Hello` with `PROTOCOL_VERSION`, a stable participant identity, a display name and, optionally, the environments it can present, each an `EnvironmentVersion` with its protocol version. The grid, which every client draws built in, is `GRID` at `GRID_VERSION`; every other environment defines its identifier and version in its own crate, such as `tomori-environment-sky`'s `ID` and `VERSION`, which its server and client halves both use. The identity is a secret of the device: it lets the server recognize a reconnect as the same participant, and is never sent to other clients.
2. The server answers `ServerMessage::Welcome` with the client's audio `source` number, its `ConnectionRole`, the environments a space may be bound to, each with its version (only those the client listed at the same version, when it listed any; a `SpaceState` names its environment's version too), the longest utterance its speech recognition takes (`utterance_limit`, in seconds, absent when there is no limit), and, for owners only, the backends and workspace roots.
3. The client lists spaces with `ListSpaces`, creates one with `CreateSpace`, or joins one with `Join`. A joined client receives `ServerMessage::Space` with the complete `SpaceState` on joining and whenever it changes, so a client can always draw the latest state without keeping history.
4. Push-to-talk sends `UtteranceBegin`, audio frames gated `Gate::Agent`, then `UtteranceEnd`; the server answers with `Transcript` messages and queues the final text for the agent. A client that sets `progress` in `UtteranceBegin` is also sent `TranscriptProgress`, on that connection, from the utterance's end until its outcome: that it is being transcribed as the end arrives, then, from recognition that says, when it waits behind another utterance (`TranscriptStage::Waiting`) and which part of several is being transcribed. A client that does not set it is never sent one, so set it only if the client reads it. Agent speech arrives as `Speech` announcements followed by audio frames from `AGENT_SOURCE`, or `REPLAY_SOURCE` when played again for one participant.
5. Approvals and questions in a `SpaceState` are answered with `Decide` and `Answer`. A failed request is reported with `ServerMessage::Error`, and the connection stays open.

## Keepalive

Any data received counts as liveness. A side that has received nothing for the `Keepalive` interval (5 seconds by default) sends `Ping` with a nonce, and the peer answers at once with `Pong` echoing it, even before Hello. A side that has received nothing for the timeout (15 seconds by default) closes the connection.

## Compatibility

- The server refuses a `Hello` with another `PROTOCOL_VERSION`, and Bonjour browsing reports such servers as incompatible. Change the version only for incompatible changes.
- New optional fields use `#[serde(default)]` and are skipped when absent, so older peers ignore them and newer peers read their absence. Document what an absent field means from an older peer.
- Enumerations that grow, such as space, worker and engagement states and pointed kinds, have an `Unknown` variant marked `#[serde(other)]`. Read a state you do not know as unknown, never as idle or finished, rather than refusing the message.
- A space's state carries its environment's `anchors`, where participants arrive, in the environment's own coordinates, and the agent's home and the workers' arrangement, in the space's, only those the environment gives, each facing along its +Z (`Pose::facing_towards`); a client uses its default layout for each that is absent, as from an older server. It carries `summoned`, where a participant summoned the agent, which a client asks for with `ClientMessage::PlaceAgent` only where `summons` is set.
- A space's state carries the `avatar` its backend names, and `BackendInfo` the same, an identifier that `valid_avatar` accepts; a client shows the swarm when it is absent, as from an older server, or names one its build lacks.
- An environment has a protocol version of its own, agreed between the server and each client side: a change an older client side can ignore keeps it, and any other change raises it. A client that lists no environments in `Hello` is offered every one, and an environment without a version is at version 1. See [`design/environments.md`](https://github.com/ioquatix/tomori/blob/main/design/environments.md).
- Send a newer message only to a server known to understand it, as the documentation of each message states, such as `Replay` only to a server whose `SpaceState::speech` is known.

The full requirements are in [`design/server.md`](https://github.com/ioquatix/tomori/blob/main/design/server.md), [`design/connection-liveness.md`](https://github.com/ioquatix/tomori/blob/main/design/connection-liveness.md) and [`design/security.md`](https://github.com/ioquatix/tomori/blob/main/design/security.md).
