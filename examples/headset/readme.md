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

Its tests draw the floor on the shared test device for a space in the placeholder environment, and in the server-less window that `tomori`'s command line opens for its build, and run the executable.

On the Quest, a headset build has its own `android_main`, which runs its builder, as this one's in `src/lib.rs` does:

```rust
#[unsafe(no_mangle)]
fn android_main(app: tomori::AndroidApp) {
    tomori_example_headset::builder().run_android(app)
}
```

The `tomori` library has an `android_main` of its own, which runs Tomori's build, behind its default `app` feature. A build turns it off by depending on `tomori-client-quest` without default features, as this one does, so that the two do not collide:

```toml
tomori-client-quest = { version = "0.1.0", default-features = false }
```

Packaging such a build as its own APK is planned in [#132](https://github.com/ioquatix/tomori/pull/132); until then this example is built for the host only.
