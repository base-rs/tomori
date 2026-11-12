// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! What the tests of pairing a device share: the server's certificate fingerprint, and a device's side of a pairing. Each test file declares this module `pub`, so that a helper it does not use is not dead code.
use tomori_protocol::{ClientMessage, ServerMessage};
use tomori_security::Fingerprint;
use tomori_security::pairing::{Nonce, code};
use tomori_server::test_support::TestClient;

/// The fingerprint of the certificate the test servers pair with.
pub fn fingerprint() -> Fingerprint {
    Fingerprint::of(b"test certificate")
}

/// Begin and reveal, as a device does, until the pairing waits for its code to be confirmed: returns the device's value and the server's.
pub fn reveal(client: &mut TestClient) -> (Nonce, Nonce) {
    let device = Nonce::generate();
    client.send(&ClientMessage::PairBegin {
        commitment: device.commitment(),
    });
    let server = client.wait_for(|message| match message {
        ServerMessage::PairNonce { nonce } => Nonce::parse(nonce),
        ServerMessage::PairFailed { error } => panic!("pairing was refused: {error}"),
        _ => None,
    });
    client.send(&ClientMessage::PairReveal {
        nonce: device.to_hex(),
    });
    client.wait_for(|message| matches!(message, ServerMessage::PairWaiting).then_some(()));
    (device, server)
}

/// Begin and reveal, as a device does, returning the code it derives and shows.
pub fn pair(client: &mut TestClient) -> String {
    let (device, server) = reveal(client);
    code(fingerprint(), device, server)
}

/// The token a pairing issued to `client`, once it is paired.
pub fn paired(client: &TestClient) -> String {
    client.wait_for(|message| match message {
        ServerMessage::Paired { token } => Some(token.clone()),
        _ => None,
    })
}

/// Why `client`'s pairing failed, once it is told.
pub fn failed(client: &TestClient) -> String {
    client.wait_for(|message| match message {
        ServerMessage::PairFailed { error } => Some(error.clone()),
        _ => None,
    })
}
