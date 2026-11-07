# `tomori-protocol`

The wire protocol shared by [Tomori](https://github.com/ioquatix/tomori) clients and the Tomori server: the control messages each side sends, the binary audio frames, and the framing that carries both over one long-lived duplex connection.

## Usage

```sh
cargo add tomori-protocol
```

A connection body in either direction is a sequence of `Frame`s, each a kind byte, a little-endian `u32` length and a payload: JSON control messages (`ClientMessage` and `ServerMessage`) or binary `AudioFrame`s of 16-bit mono PCM.

```rust
use tomori_protocol::{ClientMessage, Frame, FrameDecoder, PROTOCOL_VERSION};

let hello = ClientMessage::Hello {
    protocol: PROTOCOL_VERSION,
    participant: "a-secret-stable-identity".into(),
    name: "Console".into(),
    names: false,
    // The environments this client presents, with their versions; `None` is offered every one.
    environments: None,
};
let bytes = Frame::client(&hello).encode();

let mut decoder = FrameDecoder::new();
decoder.push(&bytes);
let frame = decoder.next_frame()?.expect("a whole frame");
assert_eq!(frame.to_client_message()?, hello);
```

Most applications use [`tomori-client`](https://crates.io/crates/tomori-client), which keeps the connection, reconnects and handles keepalive, rather than framing messages themselves. The protocol is described in [`design/server.md`](https://github.com/ioquatix/tomori/blob/main/design/server.md).

## See Also

- [Tomori](https://github.com/ioquatix/tomori), the conversational spatial workspace these crates belong to.
- [`tomori-client`](https://crates.io/crates/tomori-client), the client connection built on this protocol.
- [`tomori-server`](https://crates.io/crates/tomori-server), the server that speaks it.
