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
            Event::UtteranceFailed(failed) => eprintln!("{}", failed.failure),
            // Other messages, and whatever a later version of the client observes.
            _ => {}
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

The [`connect` example](https://github.com/ioquatix/tomori/blob/main/crates/client/examples/connect.rs) is a complete console client.

## A connection for a session

`connection::Connection` is the whole of a client's connection, as Tomori's headset, desktop window, console and spectator use it. It owns the `Client`, the microphone and the speaker, and carries out a `connection::Session`: the client's conversation with the server, which decides what to do and is told what happens, and touches neither the network nor a device. `tomori-scene`'s `Conversation` is Tomori's session; implement the trait for another.

```rust
use tomori_client::{
    Event,
    connection::{Action, Connection, Report, Session, Utterance},
    identity::{Identity, Overrides, Store},
};

#[derive(Default)]
struct Bot {
    actions: std::collections::VecDeque<Action>,
    generation: Option<u64>,
}

impl Session for Bot {
    fn next_action(&mut self) -> Option<Action> {
        self.actions.pop_front()
    }
    fn apply(&mut self, _event: Event) {
        // Server messages and connection states, as `Client::events` gives them.
    }
    fn report(&mut self, report: Report) {
        // What the connection itself says: the server to connect to at startup, the servers found, failures and security notices.
        match report {
            Report::Connecting(choice) => self.actions.push_back(Action::Connect(choice)),
            Report::Generation(generation) => self.generation = Some(generation),
            // Reports this bot has no use for, including those a later version adds.
            _ => {}
        }
    }
    // A bot shows no server list and nothing waits for it, so `is_browsing` and `observe` keep their defaults.
}

impl Bot {
    /// Say something to the agent, until `Action::EndUtterance`.
    fn speak(&mut self, id: &str) {
        if let Some(generation) = self.generation {
            let utterance = Utterance::new(id, generation);
            self.actions.push_back(Action::BeginUtterance(utterance));
        }
    }
}
```

- **Creating it:** `Connection::new(identity, store, multicast, capture, playback, &mut session)`. `identity::Identity::new(&store, device_name, overrides)` makes this device's participant identity and display name; `identity::Store::new(Some(directory))` keeps it, each server's pinned certificate and token, the last server and the Feedback setting across runs (`None` keeps nothing). `capture::Capture` and `speaker::Playback` are the platform's microphone and speaker; `()` is the multicast permission of a platform that needs none.
- **Each frame:** call `poll(focused, &mut session)`. It applies the client's `Event`s and the connection's `Report`s to the session, then takes its `Action`s one at a time, in order, so that a report such as `Report::MicrophoneFailed` can withdraw those queued behind the one that failed. `Report::Tick` comes once a frame, and `Report::Unfocused` when the application has lost focus, before the microphone closes.
- **Connecting:** a server given at startup is reported as `Report::Connecting`; the session connects with `Action::Connect`. The connection pins the certificate the server presents, presents a token given at startup only to a server given at startup or reached over USB, forgets a token the server refuses, distrusts a server whose certificate changed, and asks a server that refuses visitors to pair. See `design/security.md`.
- **Pairing and knocking:** these are the session's to carry out, as the participant asks. To pair, send `ClientMessage::PairBegin` with a commitment to a random value (`tomori_security::pairing::Nonce`), answer `ServerMessage::PairNonce` with `ClientMessage::PairReveal`, show the code derived from both values and the certificate in `ConnectionState::Connected`, and pass the token of `ServerMessage::Paired` on as `Action::Paired { token }`. Pass on the token of `ServerMessage::Admitted`, for the session's own knock, as `Action::Admitted { token }`. The connection keeps each token with the certificate this connection presented, which the session cannot choose, and ignores one it cannot keep, such as one asked for after the connection closed.
- **The microphone:** `Action::BeginUtterance(Utterance::new(id, generation))` and `Action::EndUtterance` record push-to-talk for the agent, and `Action::StartLive` and `Action::StopLive` the live microphone, each for the connection generation that `Report::Generation` gave. `Utterance::answering` makes the words the answer to a question's field instead.
- **Every action, in every state:** the connection carries out each action whatever state it is in. One that needs a connection it does not have fails with a report, such as `Report::Failed` for a message or `Report::MicrophoneFailed` for the microphone, or is ignored and logged where nothing failed that the participant should see.
- **Feedback:** `feedback()` is the participant's cues and vibrations: the frame loop reports pointing and choosing to it and takes its vibrations, and the connection plays its sound cues. `Session::observe` tells it, once a frame, what waits for the participant; by default, nothing does.
- **Voices:** `listen(participants, place)` places each other participant's voice once a frame, and `send_presence` reports this participant's head, controllers and hands to the others. The requirements the client follows are in [`design/server.md`](https://github.com/ioquatix/tomori/blob/main/design/server.md), [`design/live-audio.md`](https://github.com/ioquatix/tomori/blob/main/design/live-audio.md) and [`design/security.md`](https://github.com/ioquatix/tomori/blob/main/design/security.md).

## Types that grow

Enums such as `Event`, `ConnectionState`, `Disconnect`, `connection::Action`, `connection::Report`, `server_choice::Source` and `playback::Earcon` are `#[non_exhaustive]`: later versions may add variants, so match them with a wildcard arm for what you do not handle. Structs such as `Options`, `identity::Overrides`, `identity::Identity`, `server_choice::Choice`, `connection::Utterance` and `connection::Voices` are too: later versions may add fields, so make them with their constructor or `Default`, such as `Options::new`, `Utterance::new` or `Overrides::default()`, then set the public fields you need, and match them with `..`.
