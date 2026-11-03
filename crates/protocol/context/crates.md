# Tomori Crates

Tomori is a conversational spatial workspace for people and agents: a headset or desktop client joins a space on a Tomori server, which runs an agent session for that space, transcribes what participants say to it, and speaks its replies. Use this guide to choose which Tomori crates a project needs.

## Published crates

The crates share one version and are released together, so depend on the same version of each.

| Crate | Purpose |
| --- | --- |
| `tomori-protocol` | The wire protocol: control messages, audio frames and the framing of a connection. Every other crate builds on it. See [Using the Tomori Protocol](protocol.md). |
| `tomori-client` | A client's connection to a server, with reconnection, keepalive, microphone audio and playback. Start here to build a client. |
| `tomori-discovery` | Bonjour advertising and browsing of servers (`_tomori._tcp`). `tomori-client` re-exports it as `tomori_client::discovery`. |
| `tomori-security` | Tokens, pairing codes and certificate pinning, shared by clients and the server. The `issue` feature adds the server's token store and certificate. |
| `tomori-backend` | The backend-neutral interface between the server and coding agents: implement `Backend` and `Session` to drive another agent. |
| `tomori-backend-opencode` | The backend that drives OpenCode sessions on a running `opencode serve`. |
| `tomori-speech` | Streaming speech recognition, speakable text selection and synthesis, through prebuilt sherpa-onnx libraries. |
| `tomori-server` | The server, as the `tomori-server` executable and a library. Its interface for registering environments, data sources and tools is planned and not stable yet. |

## Unpublished crates

- `tomori-quest`, the headset and desktop application, whose library is `tomori`, is released as an Android APK rather than to crates.io.
- `tomori-environment-sky`, the renderer of the Sky environment, waits for the environment interface that will let other projects register environments.

## Choosing crates

- **A client of an existing server:** depend on `tomori-client` and `tomori-protocol`. The client presents tokens and pins certificates, and re-exports discovery; the application supplies the token, and stores the certificate fingerprint to pin.
- **Another implementation of the protocol, in Rust:** depend on `tomori-protocol` alone, and follow its framing, versioning and keepalive rules.
- **A backend for another agent:** depend on `tomori-backend`, and report structured events rather than text for display or speech.
- **A server:** run `tomori-server`. Building a customized server as a library is not supported yet.

The design documents in the [Tomori repository](https://github.com/ioquatix/tomori/tree/main/design) state the requirements these crates follow, such as `server.md` for the connection and `security.md` for authentication.
