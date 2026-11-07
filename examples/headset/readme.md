# Example Headset

An example of a headset that another project builds on [`tomori-client-quest`](../../crates/client-quest), without changing Tomori. It draws a placeholder scenery of its own, a flat coloured floor, for the placeholder environment that [the example server](../server/readme.md) offers, registered through `tomori::Builder`, beside the grid and Tomori's Sky, which it registers from [`tomori-environment-sky-client`](../../crates/environment-sky-client/readme.md), as Tomori's own app does, since the builder draws nothing but the grid until a scenery is registered.

## Usage

Its executable takes the same command line as `tomori`; the server-less window's **E** cycles through the grid, Sky and the placeholder floor:

```sh
cargo run -p tomori-example-headset
```

With the example server running, `--desktop` or `--xr` shows the floor in a space created in the Placeholder environment, which the example server offers only to clients that present it, such as this build; Tomori's own build is not offered it, and in a space someone else created in it shows the grid and names the environment and the version it lacks:

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

It is packaged as an APK of its own, `com.example.tomori.headset`, labelled **Tomori Example**, with an icon and an asset of its own. Its manifest is a copy of `tomori-client-quest`'s in its `Cargo.toml`, as [Packaging a Headset Build for the Quest](../../crates/client-quest/context/android.md) describes, and it installs beside Tomori's own app:

```sh
bash scripts/android.sh --package tomori-example-headset run
```

The APK is `target/debug/apk/tomori-example-headset.apk`. Its `assets` hold its own `tomori-example-headset.txt` and links to the licence notices in `crates/render/assets`, and its `res` its launcher icon. CI builds it on every pull request.
