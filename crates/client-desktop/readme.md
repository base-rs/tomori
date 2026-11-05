# `tomori-client-desktop`

[Tomori](https://github.com/ioquatix/tomori) on the desktop: the server-less window, taking part in a server's spaces from a window or a terminal, and finding servers on the network. The window, its event loop and the desktop's microphone and speaker are here too, for the spectator's window to use. Windows draw with Vulkan on Linux and Windows, and Metal on macOS; audio uses cpal, which needs the ALSA development files on Linux.

## Usage

The `tomori` executable, from [`tomori-client-quest`](https://crates.io/crates/tomori-client-quest), runs these modes: the server-less window by default, `--desktop` to take part, `--console` in a terminal, and `discover` to list servers.

```sh
cargo install tomori-client-quest
tomori --desktop --server 192.168.1.10:8765
```

A build of your own runs them with its own sceneries, through `tomori::Builder`'s `run_desktop`, `run_demo` and `run_console`. The `testing` feature shows a window's contents on the shared test device without a display, for other crates' tests.

Its interfaces change with the environment framework ([#75](https://github.com/ioquatix/tomori/issues/75), [#77](https://github.com/ioquatix/tomori/issues/77)).

## See Also

- [Tomori](https://github.com/ioquatix/tomori), the conversational spatial workspace these crates belong to.
- [`tomori-client-spectator`](https://crates.io/crates/tomori-client-spectator), the spectator, which uses this crate's window.
