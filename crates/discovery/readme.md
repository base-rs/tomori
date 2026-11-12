# `tomori-discovery`

Bonjour discovery of [Tomori](https://github.com/base-rs/tomori) servers: a server advertises the service type `_tomori._tcp` with its protocol version, display name and counts, and a client browses for compatible servers. It uses DNS service discovery over multicast DNS, implemented in pure Rust by `mdns-sd`, with no async runtime.

## Usage

```sh
cargo add tomori-discovery
```

```rust
use std::time::Duration;
use tomori_discovery::{Servers, browse};

let browser = browse()?;
let mut servers = Servers::new();
while let Ok(event) = browser.recv_timeout(Duration::from_secs(3)) {
    servers.apply(event);
}
for server in servers.compatible() {
    println!("{} at {:?}", server.name, server.address());
}
```

Servers whose protocol version differs from this build's are reported as incompatible, never as found. On Android, including Meta Quest, hold a Wi-Fi multicast lock while browsing; the crate documentation explains how.

## See Also

- [Tomori](https://github.com/base-rs/tomori), the conversational spatial workspace these crates belong to.
- [`tomori-client`](https://crates.io/crates/tomori-client), which re-exports this crate as `tomori_client::discovery`.
