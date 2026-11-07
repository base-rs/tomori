# `tomori-environment`

The interface of a [Tomori](https://github.com/ioquatix/tomori) environment's server side: a program in the server that runs while a space is bound to its environment, with the space it runs for. It has no other dependency on the server, so that an environment's crate can be built without it.

## Usage

```sh
cargo add tomori-environment
```

Implement `ServerSide`, and register it with its environment on the server's builder, as [`tomori-server`](https://crates.io/crates/tomori-server) describes:

```rust
use tomori_environment::{Result, ServerSide, Space};

struct Observe;

impl ServerSide for Observe {
    fn run(&self, space: &Space) -> Result<()> {
        while !space.cancellation().is_cancelled() {
            // Read what the space presents, until the space no longer needs it.
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        Ok(())
    }
}
```

A side shares what its environment presents with the clients in its space through the state channel: `Space::share` keeps a state for everyone in the space now and whoever joins later, `Space::announce` sends an event with the state it leaves, `Space::describe` describes what a participant can point at for the space's agent, and `Space::next_intent` takes what participants ask of it, which the side validates. The state, events and intents are the environment's own JSON, which the server carries without reading:

```rust
use std::time::Duration;
use tomori_environment::{Result, ServerSide, Space};

struct Counter;

impl ServerSide for Counter {
    fn run(&self, space: &Space) -> Result<()> {
        let mut count = 0;
        space.share(format!(r#"{{"count":{count}}}"#))?;
        while !space.cancellation().is_cancelled() {
            if let Some(intent) = space.next_intent(Duration::from_millis(100)) {
                count += 1;
                space.announce(
                    format!(r#"{{"by":{}}}"#, intent.participant()),
                    format!(r#"{{"count":{count}}}"#),
                )?;
            }
        }
        Ok(())
    }
}
```

[Making an Environment](context/making-an-environment.md) describes the crates an environment is made of, its common, server and client crates, and how applications register it. The interface is 0.x, and grows as the rest of the environment framework does, such as its client side. Its requirements are in Tomori's [`design/environments.md`](https://github.com/ioquatix/tomori/blob/main/design/environments.md) and [`design/builders.md`](https://github.com/ioquatix/tomori/blob/main/design/builders.md).

## See Also

- [Tomori](https://github.com/ioquatix/tomori), the conversational spatial workspace these crates belong to.
- [`tomori-server`](https://crates.io/crates/tomori-server), which runs server sides in its spaces.
