# `tomori-scene`

The platform-neutral [Tomori](https://github.com/ioquatix/tomori) client: the frame step that the headset and the desktop window share, with each platform's own logic, the conversation with a server, arrival, the hand menu, the transcript, pointing, the swarm and its workers, haptic and spatial audio logic, and the panels' layout. The conversation is the `Session` that [`tomori-client`](https://crates.io/crates/tomori-client)'s `Connection` carries out, so the connection, the microphone and speaker traits and the feedback model are that crate's.

It depends on no rendering, windowing or platform audio crate, so it builds for every client target, including `aarch64-linux-android`; a test checks that with `cargo tree`.

## Usage

```sh
cargo add tomori-scene
```

A client polls its `tomori_client::connection::Connection` with its `conversation::Conversation` once a frame, turns its devices' input into `scene::Buttons` and `scene::Senses`, runs the frame step, and draws the `scene::Frame` it returns, as [`tomori-render`](https://crates.io/crates/tomori-render)'s `scene_graphics::draw` does. Most projects build on [`tomori-client-quest`](https://crates.io/crates/tomori-client-quest)'s builder rather than on this crate directly.

The `testing` feature adds a microphone and a speaker without hardware, and a TLS test server with scripted agent sessions, for other crates' tests.

Its interfaces change with the environment framework ([#75](https://github.com/ioquatix/tomori/issues/75), [#77](https://github.com/ioquatix/tomori/issues/77)).

## See Also

- [Tomori](https://github.com/ioquatix/tomori), the conversational spatial workspace these crates belong to.
- [`tomori-client`](https://crates.io/crates/tomori-client), the connection to a server that carries out the conversation.
- [`tomori-render`](https://crates.io/crates/tomori-render), which draws its frames.
