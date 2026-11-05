# Using the Tomori Client

`tomori-client` connects a client application to a Tomori server and keeps it connected. Use this guide to build a client, such as a console, a bot that joins a space, or another headset. For the messages it carries, see the Using the Tomori Protocol guide from `tomori-protocol`.

## Connecting

Create `Options` with the server's address (`host:port`, or a host alone for port 8765), a stable participant identity and a display name, then call `Client::connect`. It returns at once: the connection runs on its own thread, with its own scheduler, so the application needs no async runtime.

```rust
use tomori_client::{Client, Event, Options, Transport};
use tomori_protocol::{ClientMessage, ServerMessage};

fn run(address: &str, participant: &str, token: Option<String>) -> std::io::Result<()> {
    let mut options = Options::new(address, participant, "Console");
    options.transport = Transport::Automatic { pin: None };
    options.token = token;
    let client = Client::connect(options)?;
    while let Ok(event) = client.events().recv() {
        match event {
            Event::State(state) if state.is_connected() => {
                let _ = client.send(ClientMessage::ListSpaces);
            }
            Event::State(state) => eprintln!("{state:?}"),
            Event::Message(ServerMessage::Spaces { spaces }) => {
                for space in spaces {
                    println!("{}: {}", space.id, space.title);
                }
            }
            Event::Message(_) => {}
            Event::UtteranceFailed(failed) => eprintln!("{}", failed.failure),
        }
    }
    Ok(())
}
```

- **Identity:** keep the participant identity stable for the device, and secret: the server recognizes a reconnect by it and restores the participant's place.
- **Transport and pinning:** a server that admits by token serves only TLS with a self-signed certificate. `Transport::Tls { pin: None }` accepts the first certificate and reports its fingerprint in `ConnectionState::Connected { certificate, .. }`; store it and pass it as the pin from then on. `Transport::Automatic` also allows plain HTTP to a server in `--auth open` mode, for an address nothing is known about yet. `Transport::Plain` never uses TLS.
- **Tokens:** set `Options::token` to an owner or collaborator token from `tomori-server auth mint`, or from pairing. It is sent only over TLS. Without one, the client is a visitor who can watch and listen.

## Events and sending

`Client::events` delivers `Event::State` for each connection state change, `Event::Message` for each `ServerMessage`, and `Event::UtteranceFailed` when push-to-talk audio could not be delivered. Received audio is read separately with `Client::try_audio`, or moved to another thread with `Client::take_audio`.

Send control messages with `Client::send`, or from other threads through `Client::sender`. The client reconnects with backoff when a connection ends, says Hello again, and rejoins the space it was in, so applications only redraw from the events. Close it with `Client::close`, which finishes sending queued control messages first.

## Finding servers

`tomori_client::discovery::browse` reports servers advertised with Bonjour on the local network, and `discovery::Servers` folds its events into a list; `Server::address` gives the address for `Options::new`. On Android, hold a Wi-Fi multicast lock while browsing, as the `tomori-discovery` documentation explains.

## Audio

- **Microphone:** create a `Microphone` with the client's `Sender` and the capture sample rate, and `push` captured samples from the audio thread. `set_gate` chooses whether others hear the participant live; `begin` and `end` send a push-to-talk utterance for the agent, and `cancel` abandons it. Its utterances ask for `ServerMessage::TranscriptProgress`, which says from the end of each until its transcript whether the server is transcribing it or waiting to.
- **Playback:** create a `playback::Player` from the receiver `take_audio` returns, `prepare` it at the device's rate, `render` it from the output device's callback in mono or stereo (`playback::Layout`), and control it from the application through its `PlayerControl`. It mixes participants and agent speech with a jitter buffer per source. For stereo, give it the listener and each source's position with `PlayerControl::set_scene`, about once a frame, and it places each voice where it is. `PlayerControl::play` plays a synthesized sound cue (`playback::Earcon`) from a `playback::Direction`, and `set_silenced` silences playback while the microphone records push-to-talk.

The [`connect` example](https://github.com/ioquatix/tomori/blob/main/crates/client/examples/connect.rs) is a complete console client. The requirements the client follows are in [`design/server.md`](https://github.com/ioquatix/tomori/blob/main/design/server.md), [`design/live-audio.md`](https://github.com/ioquatix/tomori/blob/main/design/live-audio.md) and [`design/security.md`](https://github.com/ioquatix/tomori/blob/main/design/security.md).
