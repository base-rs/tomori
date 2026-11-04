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

1. The client sends `ClientMessage::Hello` with `PROTOCOL_VERSION`, a stable participant identity and a display name. The identity is a secret of the device: it lets the server recognize a reconnect as the same participant, and is never sent to other clients.
2. The server answers `ServerMessage::Welcome` with the client's audio `source` number, its `ConnectionRole`, the environments a space may be bound to, the longest utterance its speech recognition takes (`utterance_limit`, in seconds, absent when there is no limit), and, for owners only, the backends and workspace roots.
3. The client lists spaces with `ListSpaces`, creates one with `CreateSpace`, or joins one with `Join`. A joined client receives `ServerMessage::Space` with the complete `SpaceState` on joining and whenever it changes, so a client can always draw the latest state without keeping history.
4. Push-to-talk sends `UtteranceBegin`, audio frames gated `Gate::Agent`, then `UtteranceEnd`; the server answers with `Transcript` messages and queues the final text for the agent. Agent speech arrives as `Speech` announcements followed by audio frames from `AGENT_SOURCE`, or `REPLAY_SOURCE` when played again for one participant.
5. Approvals and questions in a `SpaceState` are answered with `Decide` and `Answer`. A failed request is reported with `ServerMessage::Error`, and the connection stays open.

## Keepalive

Any data received counts as liveness. A side that has received nothing for the `Keepalive` interval (5 seconds by default) sends `Ping` with a nonce, and the peer answers at once with `Pong` echoing it, even before Hello. A side that has received nothing for the timeout (15 seconds by default) closes the connection.

## Compatibility

- The server refuses a `Hello` with another `PROTOCOL_VERSION`, and Bonjour browsing reports such servers as incompatible. Change the version only for incompatible changes.
- New optional fields use `#[serde(default)]` and are skipped when absent, so older peers ignore them and newer peers read their absence. Document what an absent field means from an older peer.
- Enumerations that grow, such as space, worker and engagement states and pointed kinds, have an `Unknown` variant marked `#[serde(other)]`. Read a state you do not know as unknown, never as idle or finished, rather than refusing the message.
- Send a newer message only to a server known to understand it, as the documentation of each message states, such as `Replay` only to a server whose `SpaceState::speech` is known.

The full requirements are in [`design/server.md`](https://github.com/ioquatix/tomori/blob/main/design/server.md), [`design/connection-liveness.md`](https://github.com/ioquatix/tomori/blob/main/design/connection-liveness.md) and [`design/security.md`](https://github.com/ioquatix/tomori/blob/main/design/security.md).
