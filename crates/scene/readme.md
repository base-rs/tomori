# `tomori-scene`

The platform-neutral [Tomori](https://github.com/ioquatix/tomori) client: the frame step that the headset and the desktop window share, with each platform's own logic, the conversation with a server, arrival, the hand menu, the transcript, pointing, travel (the locomotion an environment declares and the participant's rig), the swarm and its workers, standing where an environment's anchors and worker placement put them, the avatar a space's backend names in place of the swarm (`avatar::Avatar`), haptic and spatial audio logic, and the panels' layout. The conversation is the `Session` that [`tomori-client`](https://crates.io/crates/tomori-client)'s `Connection` carries out, so the connection, the microphone and speaker traits and the feedback model are that crate's.

It depends on no rendering, windowing or platform audio crate, so it builds for every client target, including `aarch64-linux-android`; a test checks that with `cargo tree`.

## Usage

```sh
cargo add tomori-scene
```

A client polls its `tomori_client::connection::Connection` with its `conversation::Conversation` once a frame, turns its devices' input into `scene::Buttons` and `scene::Senses`, runs the frame step, and draws the `scene::Frame` it returns, as [`tomori-render`](https://crates.io/crates/tomori-render)'s `scene_graphics::draw` does. Most projects build on [`tomori-client-quest`](https://crates.io/crates/tomori-client-quest)'s builder rather than on this crate directly.

An environment's client side, `client_side::ClientSide`, registered beside its scenery, may mark what it picks as selectable with `Picked::selectable`, and is told each frame, in its `client_side::Input`, what the participant selects with the trigger or A (`select`, one at a time, A where nothing before it in A's order takes the press), backs out of with B (`back`) and pushes the sticks to (`left_stick`, `right_stick`), only where nothing else takes them.

An object it picks may also take the right stick, with `Picked::handles(client_side::Actions::SCROLL)`: a push that begins on it, where nothing of Tomori's takes the push, is told as `Input::scroll`, a `client_side::Scroll` naming the object, with the stick's axis and its `client_side::Phase`, began once, changed while held and ended once, rather than as `right_stick`.

The `testing` feature adds a microphone and a speaker without hardware, and a TLS test server with scripted agent sessions, for other crates' tests.

Its interfaces change with the environment framework ([#75](https://github.com/ioquatix/tomori/issues/75), [#77](https://github.com/ioquatix/tomori/issues/77)).

## See Also

- [Tomori](https://github.com/ioquatix/tomori), the conversational spatial workspace these crates belong to.
- [`tomori-client`](https://crates.io/crates/tomori-client), the connection to a server that carries out the conversation.
- [`tomori-render`](https://crates.io/crates/tomori-render), which draws its frames.
