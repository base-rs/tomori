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

## Panels on its walls

A server side can put panels on the walls its environment declares with `Environment::with_walls`, for everyone in the space, through its `Space`: text panels, and charts of live data, a small bounded spec of evenly spaced points with gaps where nothing was measured. It names each by an id of its own, publishes it again to update it in place, wherever participants moved it, and retracts it when it is done:

```rust
use tomori_environment::chart::{AxisLabel, Chart, Colour, Edge, Reference, Series, SeriesStyle, Shade};
use tomori_environment::{Panel, Space};

fn pressure(space: &Space, mean: Vec<Option<f32>>, greatest: Vec<Option<f32>>) {
    let chart = Chart::new(181)
        .with_labels(vec![AxisLabel::new(0, "-15 min"), AxisLabel::new(180, "now")])
        .with_series(Series::new("mean", Colour::Teal, SeriesStyle::Line, mean))
        .with_series(Series::new("greatest", Colour::Amber, SeriesStyle::Line, greatest))
        .with_reference(Reference::new(1.0, "target", Colour::White))
        .with_shade(Shade::new(Edge::Reference { index: 0 }, Edge::Series { index: 1 }, true, Colour::Red));
    // Published as often as the side likes: one too soon after the last waits, and the latest wins.
    let _ = space.publish(
        Panel::chart("pressure", "Scaling pressure", chart)
            .on_wall(0, Some(0))
            .with_reference("chart/pressure"),
    );
}
```

The server bounds and cleans what it is given, shows a panel's updates about once a second, the latest winning, and shows a panel only while the side runs, keeping where it was for when the side publishes it again; participants move and dismiss it as any panel, and a dismissed one stays away until the side retracts it. Older clients show a chart's title and a summary of its latest values. See Tomori's `design/environments.md` and `design/projection.md`.

## Where participants arrive, the agent lives and the Tomori wall stands

An environment's registration gives fixed homes, each a pose in the environment's own coordinates, as its scenery is drawn, facing along its +Z, the way it looks, which `tomori_protocol::Pose::at(spot).facing_towards(target)` makes: where participants arrive, with `Environment::with_arrival`, and the agent's home, with `Environment::with_agent`, at least 1.4 m above the floor, beneath which the Tomori wall's home stands by default, 0.88 m below the swarm's middle, facing the way the agent faces (`tomori_protocol::entourage::wall_beneath`), or, only where the wall must stand elsewhere, as beneath an agent too high or far to read it there, the Tomori wall's, with `Environment::with_stage`; and, if it likes, the middle of its delegated workers' arrangement, with `Environment::with_workers`, otherwise the agent's home. A space's coordinates always have home, where participants arrive, at their origin facing −Z, so Tomori carries the environment's scenery, walkable surfaces, client side, anchors and walls into them; the environment's crates never see the space's coordinates. The Tomori wall stays at its home, and the agent appears at its home and stays there until a participant summons it, its wall going beneath it. Check, in the environment's tests, that the wall can be read and the agent seen from the arrival with `tomori_environment::sightlines`, the wall where `entourage::wall_beneath` stands it, on the arrival's floor, unless you give a `stage`:

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

Its client crate may let participants travel beyond the room, declaring a locomotion profile beside its scenery, in its own coordinates (see `design/locomotion.md`): walkable surfaces to teleport onto, a ceiling to fly up to, and pairs of teleport pads between floors, which Tomori draws, highlights and interprets itself, so that a participant who steps onto one, or points at it and pulls the trigger or presses A, is taken to the other, both ways:

```rust
// NAME-client: a main floor and racks 4.5 m below it, with a pad on each.
use tomori_scene::locomotion::{Locomotion, Pad, Surface};

pub fn register(sceneries: &mut tomori_render::scenery::Sceneries) {
    sceneries.register_versioned(observe::ID, observe::VERSION, my_scenery);
    let floor = Surface::rectangle(glam::Vec3::ZERO, glam::Vec2::new(20.0, 20.0));
    let racks = Surface::rectangle(glam::Vec3::new(0.0, -4.5, 0.0), glam::Vec2::new(20.0, 20.0));
    sceneries.set_locomotion(
        observe::ID,
        Locomotion::fly(vec![floor, racks], 6.0).with_pads([[
            Pad::at(glam::Vec3::new(3.0, 0.0, -2.0)).named("Main floor"),
            Pad::at(glam::Vec3::new(3.0, -4.5, -2.0)).named("Racks").facing(glam::Vec3::NEG_X),
        ]]),
    );
}
```

Each pad must stand on one of the profile's surfaces, within 20 m of home, as the anchors are; a pair with a pad that does not is ignored. A pad is 0.5 m across from its centre unless `with_radius` says otherwise, its name is what the label on the other pad says, as `Racks ↓`, and `facing` turns whoever arrives on it, who otherwise keeps facing the way they faced.

The server offers a client only the environments it presents at the same version, and a client in a space whose environment it lacks shows the grid and names it. Tomori's own server program, `tomori-server-app`, registers Sky, the grid and the farmyard, and Tomori's own app, `tomori-client-quest`'s `app` feature, registers Sky's and the farmyard's client halves, in the same way.

## The dependency rule

No core crate depends on an environment's crate, so environments can build on the core, and on one another, without a cycle; Tomori checks this, and that an environment's common and server crates build no renderer, with a test that reads `cargo metadata`. Tomori's design document, [`design/environments.md`](https://github.com/ioquatix/tomori/blob/main/design/environments.md), states the requirements, and [`design/builders.md`](https://github.com/ioquatix/tomori/blob/main/design/builders.md) the builders'.
