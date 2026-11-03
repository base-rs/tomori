# `tomori-server`

The [Tomori](https://github.com/ioquatix/tomori) server, which runs on a PC: it hosts spaces, each bound to an agent session, and serves headsets and other clients over one duplex connection each, with speech recognition and synthesis, live audio between participants, Bonjour advertising, and token authentication over TLS.

## Usage

The server links the prebuilt [sherpa-onnx](https://k2-fsa.github.io/sherpa/onnx/) libraries through [`tomori-speech`](https://crates.io/crates/tomori-speech). Outside a checkout of Tomori, their build script downloads them without checking them, so first run [`scripts/fetch-sherpa-onnx.sh`](https://github.com/ioquatix/tomori/blob/main/scripts/fetch-sherpa-onnx.sh) from a checkout, which verifies the archive, and point `SHERPA_ONNX_ARCHIVE_DIR` at its `.tools/sherpa-onnx`; or, from the root of the checkout, install with `cargo install --path crates/server --locked`, which uses it already. Then install the server and start it with the OpenCode backend, which drives a running `opencode serve`:

```sh
SHERPA_ONNX_ARCHIVE_DIR=$PWD/.tools/sherpa-onnx cargo install tomori-server
opencode serve --service &
tomori-server --backend opencode
```

Speech recognition and synthesis also need their models, which `scripts/fetch-asr-model.sh` and `scripts/fetch-tts-model.sh` fetch into a `.tools/` directory that `--speech-root` names; without it, the server runs without speech. To build without the speech libraries at all, install with `--no-default-features --features opencode`. `tomori-server --help` lists its options, and `tomori-server auth mint --owner --name NAME` makes a token for a headset or desktop client. Spaces run only beneath each `--workspace-root`, or, without one, beneath the directory the server starts in. The server and its options are described in Tomori's [readme](https://github.com/ioquatix/tomori#join-a-space-on-a-tomori-server) and [`design/server.md`](https://github.com/ioquatix/tomori/blob/main/design/server.md).

The library is the server itself, which the executable configures and runs. Its interface for registering backends, environments, data sources and tools is planned in [#73](https://github.com/ioquatix/tomori/issues/73), and is not stable yet.

### Features

- `opencode` (default): the OpenCode backend, from [`tomori-backend-opencode`](https://crates.io/crates/tomori-backend-opencode).
- `speech` (default): speech recognition and synthesis, from [`tomori-speech`](https://crates.io/crates/tomori-speech).
- `fake` (default): a scripted in-memory backend, for tests and for trying the server without an agent.
- `speech-shared`: link the speech engines from shared sherpa-onnx libraries, such as its CUDA build.

## See Also

- [Tomori](https://github.com/ioquatix/tomori), the conversational spatial workspace these crates belong to.
- [`tomori-client`](https://crates.io/crates/tomori-client), the client connection for building other clients.
