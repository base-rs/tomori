# `tomori-render`

[Tomori](https://github.com/ioquatix/tomori)'s wgpu renderers and shaders, shared by the headset, the desktop window and the spectator: the swarm and its workers, arrival, the panels, the controllers' models, and `scene_graphics::draw`, which applies a [`tomori-scene`](https://crates.io/crates/tomori-scene) frame to the `graphics::Renderer`. It holds no window, device or platform code: each client creates its own device and targets, and hands them to the renderer.

## Usage

```sh
cargo add tomori-render
```

`scenery::Scenery` is what a client draws for a space bound to an environment, natively in each eye's pass. Implement it to draw an environment of your own, and register it by the environment's identifier in `scenery::Sceneries`, or with [`tomori-client-quest`](https://crates.io/crates/tomori-client-quest)'s `tomori::Builder::scenery`. Tomori's own build registers Sky, from [`tomori-environment-sky`](https://crates.io/crates/tomori-environment-sky), and the Minimal grid is built in.

The `test-gpu` feature adds the shared device for offscreen rendering tests and WGSL validation, for other crates' tests; they run on a GPU or on Mesa's lavapipe.

Its interfaces change with the environment framework ([#75](https://github.com/ioquatix/tomori/issues/75), [#77](https://github.com/ioquatix/tomori/issues/77)).

## Licences

The Meta Quest Touch Plus controller models in `assets/controllers` are under the [MIT License](assets/controllers/LICENSE.md), and the Basis Universal transcoder's notice is in [assets/licenses](assets/licenses/basisu-LICENSE.txt). They are compiled in; an APK that includes this crate carries these notices in its assets.

## See Also

- [Tomori](https://github.com/ioquatix/tomori), the conversational spatial workspace these crates belong to.
- [`tomori-scene`](https://crates.io/crates/tomori-scene), the frames it draws.
