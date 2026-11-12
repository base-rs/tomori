# `tomori-client`

The connection between a [Tomori](https://github.com/base-rs/tomori) client and a Tomori server: one long-lived duplex connection with automatic reconnection and keepalive, live microphone audio and push-to-talk, and real-time playback of the audio it receives. It is platform-neutral, and the headset, desktop and console clients all use it.

## Usage

```sh
cargo add tomori-client tomori-protocol
```

Connect with `Options`, then read `Event`s and send `ClientMessage`s. The client runs its connection on its own thread, so the application needs no async runtime.

```rust
use tomori_client::{Client, Event, Options, Transport};
use tomori_protocol::{ClientMessage, ServerMessage};

let mut options = Options::new("192.168.1.10:8765", "a-secret-stable-identity", "Console");
options.transport = Transport::Automatic { pin: None };
options.token = std::env::var("TOMORI_TOKEN").ok();
let client = Client::connect(options)?;

while let Ok(event) = client.events().recv() {
    match event {
        Event::State(state) if state.is_connected() => client.send(ClientMessage::ListSpaces)?,
        Event::Message(ServerMessage::Spaces { spaces }) => println!("{} spaces", spaces.len()),
        _ => {}
    }
}
```

`discovery::browse` finds servers on the local network with Bonjour. `Microphone` turns captured samples into audio frames and push-to-talk utterances, and `playback::Player` mixes received audio for an output device. The [`connect` example](https://github.com/base-rs/tomori/blob/main/crates/client/examples/connect.rs) is a complete console client.

A client with Tomori's own behaviour drives a `connection::Connection` instead, once a frame, for its `connection::Session`: the client's conversation with the server, which asks for `connection::Action`s and is told what happens. The connection chooses the server, keeps its certificate and token for this device (`identity::Store`), browses for servers, records from a `capture::Capture`, plays through a `speaker::Playback` and cues what happens (`feedback`). [`tomori-scene`](https://crates.io/crates/tomori-scene)'s `Conversation` is Tomori's session.

## See Also

- [Tomori](https://github.com/base-rs/tomori), the conversational spatial workspace these crates belong to.
- [`tomori-protocol`](https://crates.io/crates/tomori-protocol), the messages and frames this client sends and receives.
- [`tomori-discovery`](https://crates.io/crates/tomori-discovery) and [`tomori-security`](https://crates.io/crates/tomori-security), for finding servers and for tokens and certificate pinning.
