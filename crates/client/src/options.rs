// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use std::time::Duration;

use tomori_protocol::Keepalive;

use crate::Backoff;

/// How a [`crate::Client`] reaches the server. A Tomori server that admits by tokens serves only TLS with a self-signed certificate, which the client pins by its fingerprint. See `design/security.md`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Transport {
    /// Plain HTTP, as to a server in `--auth open` mode. No token is presented.
    Plain,
    /// TLS, accepting only the certificate with `pin`'s fingerprint, or, without one, whichever certificate the server presents, which [`crate::ConnectionState::Connected`] reports so that it can be pinned from then on.
    Tls {
        pin: Option<tomori_security::Fingerprint>,
    },
    /// TLS as [`Self::Tls`], or plain HTTP if the server presents no certificate, for an address nothing is known about yet, such as one typed in. No token is presented over plain HTTP. With a pin, or once a connection has learned the server's certificate, it is TLS only.
    Automatic {
        pin: Option<tomori_security::Fingerprint>,
    },
}

/// How a [`crate::Client`] reaches and identifies itself to the server.
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Options {
    /// `host:port`, or a host alone for [`tomori_protocol::DEFAULT_PORT`]. Resolved again on every connection attempt.
    pub address: String,
    /// A stable identity, so the server recognizes a reconnect as the same participant. A secret of this device, which the server never sends to other clients.
    pub participant: String,
    /// The display name used for speaker attribution.
    pub name: String,
    /// A token from the server, presented on each connection, and only over TLS. See `design/security.md`.
    pub token: Option<String>,
    /// Whether to connect over TLS, and which certificate to accept.
    pub transport: Transport,
    /// Ask to be admitted only to pair, on a server that refuses visitors. See `design/security.md`.
    pub pairing: bool,
    pub backoff: Backoff,
    /// The limit for establishing the connection and for receiving the server's Welcome.
    pub connect_timeout: Duration,
    /// How long closing waits for the server to finish the connection after queued control messages are sent.
    pub close_timeout: Duration,
    /// When to ping a quiet server and when to give up on a silent one. A connection that receives nothing for [`tomori_protocol::Keepalive::timeout`] ends with [`crate::Disconnect::Timeout`] and reconnects.
    pub keepalive: Keepalive,
    /// Outgoing live audio frames queued while the link is congested; older frames beyond this are dropped. At 20 ms per frame the default is 160 ms. Frames of an open utterance are not counted or dropped; see [`Self::outgoing_utterance_limit`].
    pub outgoing_audio_limit: usize,
    /// Outgoing frames of the open utterance queued while the link is congested. They are never dropped individually, because recognition needs every one; beyond this limit the utterance fails with [`crate::UtteranceFailure::Congested`]. At 20 ms per frame the default is 30 seconds.
    pub outgoing_utterance_limit: usize,
    /// Received audio frames held for the application; newer frames are dropped when it falls this far behind.
    pub incoming_audio_limit: usize,
    /// Require a new explicit microphone activation after a connection ends.
    /// Disabled by default for clients that restore their microphone gate.
    pub microphone_rearm_on_disconnect: bool,
}

/// Written without the participant identity or the token, which are secrets, so that logging options cannot reveal them.
impl std::fmt::Debug for Options {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Options")
            .field("address", &self.address)
            .field("participant", &"[redacted]")
            .field("name", &self.name)
            .field("token", &self.token.as_ref().map(|_| "[redacted]"))
            .field("transport", &self.transport)
            .field("pairing", &self.pairing)
            .field("backoff", &self.backoff)
            .field("connect_timeout", &self.connect_timeout)
            .field("close_timeout", &self.close_timeout)
            .field("keepalive", &self.keepalive)
            .field("outgoing_audio_limit", &self.outgoing_audio_limit)
            .field("outgoing_utterance_limit", &self.outgoing_utterance_limit)
            .field("incoming_audio_limit", &self.incoming_audio_limit)
            .field(
                "microphone_rearm_on_disconnect",
                &self.microphone_rearm_on_disconnect,
            )
            .finish()
    }
}

impl Options {
    pub fn new(
        address: impl Into<String>,
        participant: impl Into<String>,
        name: impl Into<String>,
    ) -> Self {
        Self {
            address: address.into(),
            participant: participant.into(),
            name: name.into(),
            token: None,
            transport: Transport::Plain,
            pairing: false,
            backoff: Backoff::default(),
            connect_timeout: Duration::from_secs(5),
            close_timeout: Duration::from_secs(1),
            keepalive: Keepalive::default(),
            outgoing_audio_limit: 8,
            outgoing_utterance_limit: 1500,
            incoming_audio_limit: 256,
            microphone_rearm_on_disconnect: false,
        }
    }
}
