# `tomori-server`

The [Tomori](https://github.com/ioquatix/tomori) server library, which Tomori's own server, the `tomori-server` executable from [`tomori-server-app`](https://crates.io/crates/tomori-server-app), is built on, to run on a PC: it hosts spaces, each bound to an agent session, and serves headsets and other clients over one duplex connection each, with speech recognition and synthesis, live audio between participants, Bonjour advertising, and token authentication over TLS.

## Usage

The server links the prebuilt [sherpa-onnx](https://k2-fsa.github.io/sherpa/onnx/) libraries through [`tomori-speech`](https://crates.io/crates/tomori-speech). Outside a checkout of Tomori, their build script downloads them without checking them, so first run [`scripts/fetch-sherpa-onnx.sh`](https://github.com/ioquatix/tomori/blob/main/scripts/fetch-sherpa-onnx.sh) from a checkout, which verifies the archive, and point `SHERPA_ONNX_ARCHIVE_DIR` at its `.tools/sherpa-onnx`; or, from the root of the checkout, install with `cargo install --path crates/server-app --locked`, which uses it already. Then, from the checkout's root, install the server and start it with the OpenCode backend, which drives a running `opencode serve`:

```sh
SHERPA_ONNX_ARCHIVE_DIR=$PWD/.tools/sherpa-onnx cargo install tomori-server-app
opencode serve --service &
tomori-server --backend opencode
```

Speech recognition and synthesis also need their models, which `scripts/fetch-asr-model.sh` and `scripts/fetch-tts-model.sh` fetch into `.tools/` under the directory they run in, and `--speech-root` names that directory; without it, the server runs without speech. To build without the speech libraries at all, install `tomori-server-app` with `--no-default-features --features opencode`. `tomori-server --help` lists its options, and `tomori-server auth mint --owner --name NAME` makes a token for a headset or desktop client. Spaces run only in or beneath each `--workspace-root`, or, without one, in or beneath the directory the server starts in. The server and its options are described in Tomori's [readme](https://github.com/ioquatix/tomori#join-a-space-on-a-tomori-server) and [`design/server.md`](https://github.com/ioquatix/tomori/blob/main/design/server.md).

The library is the server itself, and the executable is Tomori's own build of its `Builder`, which starts with Tomori's own backends and no environments: `tomori-server-app` registers Tomori's own environments on it, each from its own crate. Another project builds a server of its own by registering its own backends and environments on `Server::builder()`, which then takes the same command line and serves the same way; see the [Building a Tomori Server](context/getting-started.md) guide and the [example server](https://github.com/ioquatix/tomori/tree/main/examples/server). An environment registered there may bring a server side that the server runs for each space bound to it; data sources and tools will be registered there once Tomori has them ([#73](https://github.com/ioquatix/tomori/issues/73)). The interface is not stable yet.

### Features

- `opencode` (default): the OpenCode backend, from [`tomori-backend-opencode`](https://crates.io/crates/tomori-backend-opencode).
- `speech` (default): speech recognition and synthesis, from [`tomori-speech`](https://crates.io/crates/tomori-speech).
- `fake` (default): a scripted in-memory backend, for tests and for trying the server without an agent.
- `highlight` (default): syntax highlighting of the agent's code panels, from [`tomori-highlight`](https://crates.io/crates/tomori-highlight), with tree-sitter and syntect; without it, code panels are plain.
- `speech-shared`: link the speech engines from shared sherpa-onnx libraries, such as its CUDA build.

## See Also

- [Tomori](https://github.com/ioquatix/tomori), the conversational spatial workspace these crates belong to.
- [`tomori-client`](https://crates.io/crates/tomori-client), the client connection for building other clients.
