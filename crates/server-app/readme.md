# `tomori-server-app`

[Tomori](https://github.com/base-rs/tomori)'s own server, the `tomori-server` executable, which runs on a PC: it hosts spaces, each bound to an agent session, and serves headsets and other clients over one duplex connection each, with speech recognition and synthesis, live audio between participants, Bonjour advertising, and token authentication over TLS.

It is the [`tomori-server`](https://crates.io/crates/tomori-server) library's builder with Tomori's own environments registered on it, through the same calls another project's server makes: Sky, by the identity in [`tomori-environment-sky`](https://crates.io/crates/tomori-environment-sky), first, then the Minimal grid, and the GitHub prototype when it is given `--github-repository`. The library registers no environment itself, so that it depends on no environment's crate.

## Usage

The server links the prebuilt [sherpa-onnx](https://k2-fsa.github.io/sherpa/onnx/) libraries through [`tomori-speech`](https://crates.io/crates/tomori-speech). Outside a checkout of Tomori, their build script downloads them without checking them, so first run [`scripts/fetch-sherpa-onnx.sh`](https://github.com/base-rs/tomori/blob/main/scripts/fetch-sherpa-onnx.sh) from a checkout, which verifies the archive, and point `SHERPA_ONNX_ARCHIVE_DIR` at its `.tools/sherpa-onnx`; or, from the root of the checkout, install with `cargo install --path crates/server-app --locked`, which uses it already:

```sh
SHERPA_ONNX_ARCHIVE_DIR=$PWD/.tools/sherpa-onnx cargo install tomori-server-app
opencode serve --service &
tomori-server --backend opencode
```

`tomori-server --help` lists its options, which [`tomori-server`'s readme](https://github.com/base-rs/tomori/blob/main/crates/server/readme.md) describes. Its features are the library's: `fake`, `opencode` and `speech` by default, and `speech-shared`.

Its tests check that it offers Sky and then the grid, and Tomori's own app's and spectator's tests check that they present every environment it offers, at the same identifier and version.

## See Also

- [Tomori](https://github.com/base-rs/tomori), the conversational spatial workspace these crates belong to.
- [`tomori-server`](https://crates.io/crates/tomori-server), the library, to build a server of your own on.
