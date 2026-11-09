# Making an Environment

A Tomori environment is what a space is bound to: what its participants see and interact with, and what its agent is told about it. Tomori's core crates, the server library and the client libraries, know no environment but the Minimal grid, which every client draws built in. Every other environment, Tomori's own Sky included, is made in crates of its own and registered by the applications that offer and present it, so that adding one changes nothing in Tomori. Use this guide to make one.

## Its crates

An environment is up to three crates, named for it:

- **Common**, `NAME`, such as `tomori-environment-sky`: what its halves share. At least its identity: the identifier spaces name it by, its protocol version and its label. Later, its message formats and any simulation both halves run. It must not depend on a renderer or the GPU stack, such as `tomori-render` or wgpu, so that a server builds it.
- **Server**, `NAME-server`: what runs only in the server, such as its server side, an implementation of this crate's `ServerSide`, its data sources and its credentials. A credential, such as an API token, lives here and nowhere else: never in the common crate, which clients build, nor in what the server sends. It must not depend on a renderer either.
- **Client**, `NAME-client`, such as `tomori-environment-sky-client`: how a client presents it, such as a scenery, an implementation of [`tomori-render`](https://crates.io/crates/tomori-render)'s `Scenery`, with a `register` function that registers it on a client's `Sceneries` at the common crate's identity. It may depend on the renderer.

Leave out a crate an environment has no use for: Sky has no server half. The GitHub prototype has all three: `tomori-environment-github`, its identity; `tomori-environment-github-server`, which holds its GitHub client and token and adds its `--github-repository` option to a server's builder; and `tomori-environment-github-client`, which presents it over the grid. The farmyard has a common crate and a client crate: its server side, which runs a flock of chickens that its clients replay with the same functions, holds nothing that must stay in the server, so it is in `tomori-environment-farmyard`, its common crate, which depends on this crate but not on `tomori-server`; `tomori-environment-farmyard-client` draws the yard and the chickens.

```rust
// NAME: the common crate.
pub const ID: &str = "observe";
pub const VERSION: u32 = 1;
pub const LABEL: &str = "Observe";
```

```rust
// NAME-client: its scenery, registered at its identity.
pub fn register(sceneries: &mut tomori_render::scenery::Sceneries) {
    sceneries.register_versioned(observe::ID, observe::VERSION, my_scenery);
}
```

Raise the version in the common crate, and change both halves together, when a client of the earlier version could not present the environment as the server now offers it; a change an older client can ignore keeps it.

## Registering it

An application registers each environment it offers or presents, through the same calls Tomori's own programs make:

```rust
// A server, on tomori-server's builder.
let server = tomori_server::Server::builder().environment(
    tomori_server::Environment::new(observe::ID, observe::LABEL)
        .with_version(observe::VERSION)
        .with_server_side(observe_server::Side::new()),
);
```

```rust
// A headset, on tomori-client-quest's builder.
let headset = tomori::Builder::default().register(observe_client::register);
```

## Where participants arrive, the agent lives and the Tomori wall stands

An environment's registration gives three fixed homes, each a pose in the environment's own coordinates, as its scenery is drawn, facing along its +Z, the way it looks, which `tomori_protocol::Pose::at(spot).facing_towards(target)` makes: where participants arrive, with `Environment::with_arrival`, the agent's home, with `Environment::with_agent`, and the Tomori wall's, with `Environment::with_stage`; and, if it likes, the middle of its delegated workers' arrangement, with `Environment::with_workers`, otherwise the agent's home. A space's coordinates always have home, where participants arrive, at their origin facing −Z, so Tomori carries the environment's scenery, walkable surfaces, client side, anchors and walls into them; the environment's crates never see the space's coordinates. The Tomori wall stays at its home, and the agent appears at its home and stays there until a participant summons it. Check, in the environment's tests, that the wall can be read and the agent seen from the arrival with `tomori_environment::sightlines`:

```rust
use tomori_environment::sightlines::{Spot, readable, seen};

let arrival = Spot::facing([0.0, 0.0, 8.0], [0.0, 3.0, 0.0]);
let eyes = arrival.eyes();
assert_eq!(readable(&arrival, &Spot::facing([1.5, 1.0, 6.0], eyes), &[]), Ok(()));
assert_eq!(seen(&arrival, &Spot::facing([0.0, 3.0, 5.0], eyes), &[]), Ok(()));
```

Its client crate may place the workers about the workers anchor by an algorithm of its own, an implementation of [`tomori-scene`](https://crates.io/crates/tomori-scene)'s `anchors::WorkerPlacement`, registered beside its scenery:

```rust
// NAME-client: ring the workers about the workers anchor, one per slot.
struct Ring;

impl tomori_scene::anchors::WorkerPlacement for Ring {
    fn place(&self, origin: tomori_scene::anchors::Anchor, worker: &tomori_scene::anchors::Placing<'_>) -> glam::Vec3 {
        let angle = worker.slot as f32 * 0.5;
        origin.place(glam::Vec3::new(angle.sin() * 2.0, 1.4, -angle.cos() * 2.0))
    }
}

pub fn register(sceneries: &mut tomori_render::scenery::Sceneries) {
    sceneries.register_versioned(observe::ID, observe::VERSION, my_scenery);
    sceneries.set_worker_placement(observe::ID, Ring);
}
```

A placement is asked only as a worker arrives, or when the arrangement changes, told of the workers already standing in the order of their slots, so live workers never move as others come and go; without one, the workers stand on Tomori's default arc about the anchor, which faces whoever watches it as the agent does. It runs on the frame step's thread, so keep it quick; one that panics, or answers a place that is not finite or beyond 90 m, leaves its worker on the arc. It may stand workers together: their signs then stack above one another, and beside within 30°. See [`design/environments.md`](https://github.com/ioquatix/tomori/blob/main/design/environments.md).

The server offers a client only the environments it presents at the same version, and a client in a space whose environment it lacks shows the grid and names it. Tomori's own server program, `tomori-server-app`, registers Sky, the grid and the farmyard, and Tomori's own app, `tomori-client-quest`'s `app` feature, registers Sky's and the farmyard's client halves, in the same way.

## The dependency rule

No core crate depends on an environment's crate, so environments can build on the core, and on one another, without a cycle; Tomori checks this, and that an environment's common and server crates build no renderer, with a test that reads `cargo metadata`. Tomori's design document, [`design/environments.md`](https://github.com/ioquatix/tomori/blob/main/design/environments.md), states the requirements, and [`design/builders.md`](https://github.com/ioquatix/tomori/blob/main/design/builders.md) the builders'.
