# `tomori-render`

[Tomori](https://github.com/base-rs/tomori)'s wgpu renderers and shaders, shared by the headset, the desktop window and the spectator: the swarm and its workers, arrival, the panels, the controllers' models, and `scene_graphics::draw`, which applies a [`tomori-scene`](https://crates.io/crates/tomori-scene) frame to the `graphics::Renderer`. It holds no window, device or platform code: each client creates its own device and targets, and hands them to the renderer.

## Usage

```sh
cargo add tomori-render
```

`scenery::Scenery` is what a client draws for a space bound to an environment, natively in each eye's pass. Implement it to draw an environment of your own, and register it by the environment's identifier in `scenery::Sceneries`, or with [`tomori-client-quest`](https://crates.io/crates/tomori-client-quest)'s `tomori::Builder::scenery`, at the environment's protocol version with `Sceneries::register_versioned` or `Builder::versioned_scenery`. `Sceneries::set_locomotion`, or `tomori::Builder::locomotion`, declares how participants travel in a space bound to the environment, teleporting onto walkable surfaces or flying, while its scenery is drawn. `Sceneries::set_worker_placement`, or `tomori::Builder::worker_placement`, places the space's delegated workers by the environment's own `tomori-scene` `WorkerPlacement`, about the workers anchor its server gives, while its scenery is drawn. `Sceneries::set_avatar`, or `tomori::Builder::avatar`, shows the agent as an avatar of the project's own, a `tomori-scene` `Avatar` with a `Scenery` that draws it, in a space whose backend names it, rather than as the swarm, which is built in. `Sceneries::present_over_grid` presents an environment with no scenery of its own over the grid, as the GitHub prototype's cards are. `Sceneries` starts with nothing but the Minimal grid, which is built in: each environment's client crate registers itself, as [`tomori-environment-sky-client`](https://crates.io/crates/tomori-environment-sky-client)'s `register` does Sky, and an application registers the environments it presents.

A scenery is made for a `scenery::Target`, built with `Target::new`, which carries the renderer's glyph atlas, `glyphs::SharedGlyphs`, for a scenery's own text, drawn with `ui_panel::Pipeline::shared`; `Scenery::prepare_eye` tells it where each eye is, from `scenery::eye`, and is `Scenery::prepare` by default.

Once a frame, `Scenery::submit` adds what a scenery draws to the renderer's render world, `world`, a re-export of [`tomori-world`](https://crates.io/crates/tomori-world), in its own coordinates and with its bounds, and `Scenery::cull` then draws only what `scenery::Views::seen` says the eyes may see of it; `scenery::Stage` culls a scenery alone, for its tests. A scenery that submits nothing may cull itself against `Views::frustum`, or draw everything.

The `test-gpu` feature adds the shared device for offscreen rendering tests and WGSL validation, for other crates' tests; they run on a GPU or on Mesa's lavapipe.

Its interfaces change with the environment framework ([#75](https://github.com/base-rs/tomori/issues/75), [#77](https://github.com/base-rs/tomori/issues/77)).

## Licences

The Meta Quest Touch Plus controller models in `assets/controllers` are under the [MIT License](assets/controllers/LICENSE.md), the Basis Universal transcoder's notice is in [assets/licenses](assets/licenses/basisu-LICENSE.txt), and the licences of the fonts it draws text with, from [`tomori-text`](https://crates.io/crates/tomori-text), are in [assets/licenses](assets/licenses/fonts-LICENSE.txt) too. They are compiled in; an APK that includes this crate carries these notices in its assets.

## See Also

- [Tomori](https://github.com/base-rs/tomori), the conversational spatial workspace these crates belong to.
- [`tomori-scene`](https://crates.io/crates/tomori-scene), the frames it draws.
