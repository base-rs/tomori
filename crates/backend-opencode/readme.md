# `tomori-backend-opencode`

The [OpenCode](https://opencode.ai) backend for the [Tomori](https://github.com/ioquatix/tomori) server. It drives sessions on a running `opencode serve` process over its HTTP and server-sent events APIs, and reconnects when the service restarts.

## Usage

```sh
cargo add tomori-backend-opencode
```

`OpenCode::new(Config::default())` finds the service from the `OPENCODE_URL` environment variable, then the registration file that `opencode serve --service` writes, then `http://127.0.0.1:4096`; set `Config::url` to name one explicitly. It implements `tomori_backend::Backend`, under the name `opencode`. Permissions and questions are answered only when a participant acts. See [`design/agent-backends.md`](https://github.com/ioquatix/tomori/blob/main/design/agent-backends.md).

## See Also

- [Tomori](https://github.com/ioquatix/tomori), the conversational spatial workspace these crates belong to.
- [`tomori-backend`](https://crates.io/crates/tomori-backend), the interface it implements.
- [`tomori-server`](https://crates.io/crates/tomori-server), which uses it for `--backend opencode`.
