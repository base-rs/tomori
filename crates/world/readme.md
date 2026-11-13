# `tomori-world`

[Tomori](https://github.com/base-rs/tomori)'s render world, free of GPU code: what is drawn each frame, with its bounds, layers, lights and views, and what the eyes may see of it.

A `World` is kept across frames. Its roots are the coordinates things are drawn in, each carried into the views' own; its items are what is drawn, by mesh and material ids its backend gives them, in a layer, static or dynamic, once or as many instances, each with its bounds; it holds lights and each root's environment light, and lights each instance seen by at most 8 of its root's lights, of at most 64, by one function, `assign`; and this frame's eyes, two on a headset or one on a desktop window. Once a frame it culls against one frustum enclosing every eye: static instances through a bounding volume hierarchy, built again only when they change, and dynamic ones one by one. It chooses the level of detail each instance seen is drawn in, of an item's coarser levels, each with how far it strays: the coarsest whose error spans at most a pixel from the nearer eye, alike for both eyes, with hysteresis, faded from one level to the next over a few frames. Once what is placed stops growing, a frame allocates nothing.

## Usage

```sh
cargo add tomori-world
```

```rust
use glam::{Mat4, Vec3};
use tomori_world::{Aabb, Item, Layer, Mobility, World};

let mut world = World::new();
let scenery = world.add_root();
let hills = world.add(Item::new(scenery, Layer::Opaque, Mobility::Static));
world.place(hills, [Some(Aabb::new(Vec3::new(-1.0, 0.0, -9.0), Vec3::new(1.0, 2.0, -7.0)))]);

let eye = glam::camera::rh::proj::directx::perspective(1.6, 1.0, 0.05, 2_000.0)
    * Mat4::from_translation(Vec3::new(0.0, 1.6, 0.0)).inverse();
world.set_views(&[eye]);
world.cull();
assert_eq!(world.seen(hills), [true]);
```

`tomori-render` draws what it sees, its own cubes and panels and what an environment's scenery submits through `Scenery::submit`, which it re-exports this crate for, as `tomori_render::world`. Its requirements are in Tomori's [`design/rendering.md`](https://github.com/base-rs/tomori/blob/main/design/rendering.md), and the plan it belongs to in [`docs/proposals/rendering-architecture.md`](https://github.com/base-rs/tomori/blob/main/docs/proposals/rendering-architecture.md). The interface is 0.x.

## See Also

- [Tomori](https://github.com/base-rs/tomori), the conversational spatial workspace these crates belong to.
- [`tomori-assets`](https://crates.io/crates/tomori-assets), whose boxes and frustums it culls with.
