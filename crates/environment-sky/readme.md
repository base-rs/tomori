# `tomori-environment-sky`

The identity of [Tomori](https://github.com/base-rs/tomori)'s Sky environment, a moonlit sky over a platform and an open shrine: its identifier, `ID`, protocol version, `VERSION`, and label, `LABEL`. It is Sky's common crate, which its server and client halves both use, so that the two cannot disagree; it has no dependencies and no renderer, so a server offers Sky without building one.

- Sky's renderer, and its registration with a headset build, is [`tomori-environment-sky-client`](https://crates.io/crates/tomori-environment-sky-client).
- Sky has no server side: a server offers it by registering it with [`tomori-server`](https://crates.io/crates/tomori-server)'s builder.

```rust
use tomori_server::{Environment, Server};

let builder = Server::builder().environment(
    Environment::new(tomori_environment_sky::ID, tomori_environment_sky::LABEL)
        .with_version(tomori_environment_sky::VERSION),
);
```

Its interface is 0.x. Tomori's environments follow the common, server and client convention that its [`design/environments.md`](https://github.com/base-rs/tomori/blob/main/design/environments.md) describes.
