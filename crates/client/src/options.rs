// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use std::time::Duration;

use tomori_protocol::{EnvironmentVersion, Keepalive, Limits};

use crate::Backoff;

/// Whose voices a client plays: everyone's, the agent's speech alone, live or played again, or none. A participant's
/// voice not played is dropped as it arrives, before its samples are decoded; the agent's speech, not played, is muted
/// rather than dropped, so that it is still measured and the agent is still seen speaking. The spectator chooses, so that
/// a demonstration's own voice does not come back out of its speakers. See `design/spectator.md`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum Hearing {
    /// Every participant's voice and the agent's speech.
    #[default]
    All,
    /// The agent's speech alone.
    Agent,
    /// No voice at all.
    Nobody,
}

impl Hearing {
    /// Every choice, in the order [`Self::next`] cycles through them.
    pub const EVERY: [Self; 3] = [Self::All, Self::Agent, Self::Nobody];

    /// Its name on a command line: `all`, `agent` or `none`.
    pub fn name(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Agent => "agent",
            Self::Nobody => "none",
        }
    }

    /// The choice [`Self::name`] names, if any.
    pub fn named(name: &str) -> Option<Self> {
        Self::EVERY
            .into_iter()
            .find(|hearing| hearing.name() == name)
    }

    /// The next choice: everyone's, then the agent's alone, then none, and everyone's again.
    pub fn next(self) -> Self {
        match self {
            Self::All => Self::Agent,
            Self::Agent => Self::Nobody,
            Self::Nobody => Self::All,
        }
    }

    /// Whether the participants' voices are delivered to play.
    pub(crate) fn participants(self) -> bool {
        self == Self::All
    }

    /// Whether the agent's speech is muted.
    pub(crate) fn mutes_agent(self) -> bool {
        self == Self::Nobody
    }
}

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
    /// Whose voices are delivered to play, from the first frame received; [`crate::Client::hear`] changes it. Everyone's,
    /// by default.
    pub hearing: Hearing,
    /// Require a new explicit microphone activation after a connection ends.
    /// Disabled by default for clients that restore their microphone gate.
    pub microphone_rearm_on_disconnect: bool,
    /// The environments this client presents, each at its protocol version, listed in Hello so that the server offers
    /// it only those. `None`, the default, lists nothing, and the server offers every environment, as to a client from
    /// before versions. See `design/environments.md`.
    pub environments: Option<Vec<EnvironmentVersion>>,
    /// Whether the session applies patches to the projection it holds, as `ServerMessage::ProjectionPatch` sends them,
    /// asking with `ClientMessage::ResyncProjection` for the projection whole when one does not apply, which each
    /// connection says in its Hello. `false`, the default, is sent the projection whole at every change, as a session
    /// that does not apply them needs. See `design/projection.md`.
    pub projection_patches: bool,
    /// Whether the session, which applies patches, also applies their data ops to a live panel's data, as
    /// `ProjectionPatch::apply` does, and keeps the version a place op carries, which each connection says in its Hello.
    /// `false`, the default, is sent a live panel whole at each change of its data. See `design/projection.md`.
    pub live_panels: bool,
    /// Whether the session arrives where the server says as it joins a space, `ServerMessage::Spawn`, which each
    /// connection says in its Hello. `false`, the default, is never told, and arrives at home, as the spectator, which
    /// chooses where it watches from itself. See `design/environments.md`.
    pub spawns: bool,
    /// What this client reads and draws at most, which each connection states in its Hello: by default frames of up to
    /// [`tomori_protocol::MOST_FRAME_BYTES`], which the connection reads, and a projection within today's bounds,
    /// [`tomori_protocol::Limits::TODAY`], since what a session draws is its own to say; see
    /// [`crate::connection::Session::limits`]. The connection reads frames of up to the lesser of this and what the
    /// server states in its Welcome, today's from a server that states none. See `design/projection.md`.
    pub limits: Limits,
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
            .field("hearing", &self.hearing)
            .field(
                "microphone_rearm_on_disconnect",
                &self.microphone_rearm_on_disconnect,
            )
            .field("environments", &self.environments)
            .field("projection_patches", &self.projection_patches)
            .field("live_panels", &self.live_panels)
            .field("spawns", &self.spawns)
            .field("limits", &self.limits)
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
            hearing: Hearing::All,
            microphone_rearm_on_disconnect: false,
            environments: None,
            projection_patches: false,
            live_panels: false,
            spawns: false,
            limits: Limits::TODAY.with_frame_bytes(tomori_protocol::MOST_FRAME_BYTES as u32),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_hearing_is_named_cycles_and_says_what_it_plays() {
        for hearing in Hearing::EVERY {
            assert_eq!(Hearing::named(hearing.name()), Some(hearing));
        }
        assert_eq!(Hearing::named("some"), None);
        assert_eq!(
            Hearing::EVERY.map(Hearing::next),
            [Hearing::Agent, Hearing::Nobody, Hearing::All]
        );
        assert_eq!(
            Hearing::EVERY.map(Hearing::participants),
            [true, false, false]
        );
        assert_eq!(
            Hearing::EVERY.map(Hearing::mutes_agent),
            [false, false, true]
        );
        assert_eq!(Hearing::default(), Hearing::All);
    }
}
