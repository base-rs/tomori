# `tomori-physics`

Rigid-body physics for a [Tomori](https://github.com/ioquatix/tomori) environment's loose objects, which participants take with either hand, carry, throw and stack: simulated once, by the environment's server side, on [Rapier](https://rapier.rs), and drawn alike by every client, which never simulates.

Whoever holds a body controls it, every client drawing it from their hand, which presence already brings; let go, the server side simulates it, saying where it is 20 times a second while it moves and once where it rests, and nothing while it rests. A body lost, left in water that takes it back, or sent home fades out where it is and in where it returns to, never seen on its way.

## Usage

```sh
cargo add tomori-physics
```

An environment's common crate declares its bodies, what they rest on and its water, and its server side runs the world of them, with the default `simulation` feature:

```rust
use glam::{Quat, Vec3};
use tomori_physics::world::World;
use tomori_physics::{Body, Placement, Settings, Static};

let ground = Static::cuboid(Vec3::new(-5.0, -1.0, -5.0), Vec3::new(5.0, 0.0, 5.0), Quat::IDENTITY);
let stone = Body::convex(
    0,
    vec![
        Vec3::new(0.1, -0.04, 0.0),
        Vec3::new(-0.1, -0.04, 0.05),
        Vec3::new(0.0, -0.04, -0.1),
        Vec3::new(0.0, 0.04, 0.0),
    ],
)
.at(Placement::at(Vec3::new(0.0, 0.2, 0.0)));
let mut world = World::new(Settings::default(), vec![ground], Vec::new(), vec![stone]).unwrap();
assert!(world.settle(10.0));
assert!(world.placement(0).unwrap().position.y < 0.1);
```

Its client crate depends on it without its default features, and mirrors what the side shares and announces with `mirror::Mirror`.

See Tomori's `design/physics.md`.

## See Also

- [Tomori](https://github.com/ioquatix/tomori), the conversational spatial workspace these crates belong to.
- [`tomori-environment`](https://crates.io/crates/tomori-environment), whose server sides simulate with it.
