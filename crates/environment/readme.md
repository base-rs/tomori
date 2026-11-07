# `tomori-environment`

The interface of a [Tomori](https://github.com/ioquatix/tomori) environment's server side: a program in the server that runs while a space is bound to its environment, with the space it runs for. It has no other dependency on the server, so that an environment's crate can be built without it.

## Usage

```sh
cargo add tomori-environment
```

Implement `ServerSide`, and register it with its environment on the server's builder, as [`tomori-server`](https://crates.io/crates/tomori-server) describes:

```rust
use tomori_environment::{ServerSide, Space};

struct Observe;

impl ServerSide for Observe {
    fn run(&self, space: &Space) -> Result<(), String> {
        while !space.cancellation().is_cancelled() {
            // Read what the space presents, until the space no longer needs it.
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        Ok(())
    }
}
```

The interface is 0.x, and grows as the rest of the environment framework does, such as its state, intents and client side. Its requirements are in Tomori's [`design/environments.md`](https://github.com/ioquatix/tomori/blob/main/design/environments.md) and [`design/builders.md`](https://github.com/ioquatix/tomori/blob/main/design/builders.md).

## See Also

- [Tomori](https://github.com/ioquatix/tomori), the conversational spatial workspace these crates belong to.
- [`tomori-server`](https://crates.io/crates/tomori-server), which runs server sides in its spaces.
