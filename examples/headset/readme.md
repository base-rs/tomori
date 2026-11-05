# Example Headset

An example of a headset that another project builds on [`tomori-client-quest`](../../crates/client-quest), without changing Tomori. It draws a placeholder scenery of its own, a flat coloured floor, for the placeholder environment that [the example server](../server/readme.md) offers, registered through `tomori::Builder`, beside Tomori's own Sky and grid.

## Usage

Its executable takes the same command line as `tomori`; the server-less window's **E** cycles through the grid, Sky and the placeholder floor:

```sh
cargo run -p tomori-example-headset
```

With the example server running, `--desktop` or `--xr` shows the floor in a space created in the Placeholder environment, where Tomori's own build shows the grid and names the environment it cannot draw:

```sh
cargo run -p tomori-example-server -- --backend placeholder --auth open --no-advertise
cargo run -p tomori-example-headset -- --desktop --server 127.0.0.1:8765
```

Its tests draw the floor on the shared test device for a space in the placeholder environment, and run the executable.

A headset build for the Quest has its own `android_main`, which calls `tomori_example_headset::builder().run_android(app)`. Packaging such a build as its own APK is planned in [#132](https://github.com/ioquatix/tomori/pull/132).
