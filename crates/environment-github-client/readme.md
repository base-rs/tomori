# `tomori-environment-github-client`

The client half of [Tomori](https://github.com/base-rs/tomori)'s GitHub environment prototype: `register` presents it over the grid, at the identity in [`tomori-environment-github`](https://crates.io/crates/tomori-environment-github), so that a client lists it in its `Hello` and shows a space in it without a notice, while [`tomori-scene`](https://crates.io/crates/tomori-scene) draws the cards a server sends a space that shows them. It never holds the server's GitHub token. Tomori's own app and spectator register it:

```rust
let builder = tomori::Builder::default().register(tomori_environment_github_client::register);
```

Its interface is 0.x; see Tomori's [`design/github.md`](https://github.com/base-rs/tomori/blob/main/design/github.md).
