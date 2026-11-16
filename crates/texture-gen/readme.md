# `tomori-texture-gen`

[Tomori](https://github.com/base-rs/tomori)'s tiling surface textures, generated in code from a seed: earth, stone and bark, each a base colour and a tangent-space normal map, seamless, with every mip level down to one texel made in linear light, compressed to ASTC's 4×4 blocks, which the Quest's GPU samples, and to BC7, which desktop GPUs sample, each written as KTX2.

Everything is made from integer hashes and arithmetic, never the platform's transcendental functions, so the same recipe gives the same bytes on every machine, and a texture can be named in code by the digest of its bytes, as Tomori's content store serves it.

## Usage

```sh
cargo add tomori-texture-gen
```

```rust
use tomori_texture_gen::{Format, Recipe, Surface};

let earth = Recipe::new(Surface::Earth, 64, 1).generate();
let quest = earth.color.ktx2(Format::Astc, true);
let desktop = earth.normal.ktx2(Format::Bc7, false);
assert!(quest.starts_with(&tomori_texture_gen::KTX2) && desktop.len() > 64 * 64);
// The mean of its base colour, linear, which a renderer can draw in its place.
assert!(earth.flat.iter().all(|channel| *channel > 0.0));
```

An environment's server generates its own and serves them by digest, from Tomori's content store. Its requirements are in Tomori's [`design/rendering.md`](https://github.com/base-rs/tomori/blob/main/design/rendering.md). The interface is 0.x.

## See Also

- [Tomori](https://github.com/base-rs/tomori), the conversational spatial workspace these crates belong to.
- [`tomori-render`](https://crates.io/crates/tomori-render), which loads and samples them.
