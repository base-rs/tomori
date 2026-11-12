# `tomori-assets`

Assets for [Tomori](https://github.com/base-rs/tomori)'s environments, loaded at runtime rather than baked into code: content-addressed references to them, a library that loads them from the app's bundled sources, a reader of binary glTF 2.0 (`.glb`) that treats every file as untrusted, the models it reads, instances of them with poses and nodes shown or hidden, and, with the default `gpu` feature, drawing them with wgpu, skinned on the GPU, every instance of a primitive in one draw.

## Usage

```sh
cargo add tomori-assets
```

An environment names each asset beside the code that uses it, by its environment, version, name and SHA-256 digest, adds where it is bundled to the program's library, and loads it, sharing one model between every instance:

```rust
use tomori_assets::{AssetRef, Digest, Embedded, Instance, Library};

const LANTERN: &[u8] = include_bytes!("../assets/models/lantern.glb");

let lantern = AssetRef::new("garden", 1, "lantern.glb", Digest::parse("…").unwrap());
Library::shared().add(Embedded::new().with(LANTERN));
let model = Library::shared().model(&lantern)?;
let mut instance = Instance::new(model);
instance.set_shown("glass", false);
```

A file whose bytes are not the digest's is never used, and one that is broken, hostile or larger than the `Limits` is refused with an `Error`, never a panic or an unbounded allocation. Geometry built in code is a `Model` too, made with `Model::built` and checked the same way, so that a procedural model and one read from a file are drawn alike, and either can replace the other. An environment's server side, which draws nothing, depends on it with `default-features = false`.

[`context/usage.md`](context/usage.md) describes each layer and its API. The interface is 0.x, and grows with Tomori's environments. Its requirements are in Tomori's [`design/assets.md`](https://github.com/base-rs/tomori/blob/main/design/assets.md).

## See Also

- [Tomori](https://github.com/base-rs/tomori), the conversational spatial workspace these crates belong to.
- [`tomori-render`](https://crates.io/crates/tomori-render), whose `Scenery` an environment draws its models in.
