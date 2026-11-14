# `tomori-client-spectator`

The [Tomori](https://github.com/base-rs/tomori) spectator: a read-only view of a live space that follows a participant, for OBS and calls, in a window, decorated or borderless, as a PipeWire video source without a window, or both.

## Usage

```sh
cargo install tomori-client-spectator
tomori-spectator --server 192.168.1.10:8765 --follow Alex
tomori-spectator --borderless --server 192.168.1.10:8765
tomori-spectator --pipewire --server 192.168.1.10:8765
```

`tomori-spectator --help` lists its options. Its window is decorated, to move, resize or make full screen; `--borderless` shows it without a title bar or border, for a clean capture. `--pipewire` publishes a video source, **Tomori-Spectator**, through GStreamer's `pipewiresink`, on Linux. `tomori --spectator…`, from [`tomori-client-quest`](https://crates.io/crates/tomori-client-quest), runs it too, drawing that build's sceneries. The library draws the sceneries its caller gives it, and the grid; the `tomori-spectator` executable gives it Tomori's own, `app::sceneries()`, behind the default `app` feature, which a project of its own turns off with `default-features = false`. The [Slack huddle guide](https://github.com/base-rs/tomori/blob/main/docs/slack-huddle.md) shows it in a call.

Its interfaces change with the environment framework ([#75](https://github.com/base-rs/tomori/issues/75), [#77](https://github.com/base-rs/tomori/issues/77)).

## See Also

- [Tomori](https://github.com/base-rs/tomori), the conversational spatial workspace these crates belong to.
- [`tomori-client-desktop`](https://crates.io/crates/tomori-client-desktop), whose window and speaker it uses.
