// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use std::fmt;
use std::time::Duration;

use tomori_protocol::ServerMessage;

use crate::FailedUtterance;

/// Why a connection ended or could not be established.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Disconnect {
    /// The server address could not be resolved.
    Address(String),
    /// Connecting, reading or writing failed.
    Transport(String),
    /// The server answered `/connect` with an unsuccessful status.
    Status(u16),
    /// The server sent something that violates the protocol, including an incompatible version.
    Protocol(String),
    /// Connecting or the server's Welcome took too long, or an established connection received nothing for [`tomori_protocol::Keepalive::timeout`].
    Timeout,
    /// The server presented a different certificate from the one pinned, with this fingerprint. Nothing was sent to it.
    Certificate(String),
    /// The server ended the connection.
    Closed,
}

impl fmt::Display for Disconnect {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Address(error) => write!(formatter, "cannot resolve the server address: {error}"),
            Self::Transport(error) => write!(formatter, "connection failed: {error}"),
            Self::Status(status) => write!(formatter, "server responded with status {status}"),
            Self::Protocol(error) => write!(formatter, "protocol error: {error}"),
            Self::Timeout => write!(formatter, "timed out waiting for the server"),
            Self::Certificate(fingerprint) => write!(
                formatter,
                "the server's certificate has changed (it presented {fingerprint})"
            ),
            Self::Closed => write!(formatter, "server closed the connection"),
        }
    }
}

impl std::error::Error for Disconnect {}

/// The client's connection to the server.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ConnectionState {
    /// Opening a connection. `attempt` counts from one and resets after a connection that stayed up for [`crate::Backoff::stable_duration`].
    Connecting { attempt: u32 },
    /// The server has welcomed this client. Membership has been restored.
    Connected {
        /// This client's participant number, which other clients see as the audio frame source.
        source: u32,
        server: String,
        /// What this connection may do in the lobby. A [`tomori_protocol::ServerMessage::Role`] follows whenever it changes. See `design/security.md`.
        role: tomori_protocol::ConnectionRole,
        /// The fingerprint of the server's TLS certificate, to pin from then on, or `None` over plain HTTP.
        certificate: Option<tomori_security::Fingerprint>,
        /// The server's backends, so the client shows only actions each supports. Empty unless this connection is an owner.
        backends: Vec<tomori_protocol::BackendInfo>,
        /// The directories a space may run in, so the client can choose one without typing a path.
        roots: Vec<tomori_protocol::WorkspaceRoot>,
        /// The environments a space may be bound to, so the client can choose one when creating a space.
        environments: Vec<tomori_protocol::EnvironmentInfo>,
        /// The longest utterance the server's speech recognition takes, when it has a limit, so that recording can warn before it.
        utterance_limit: Option<Duration>,
    },
    /// The connection ended; another attempt follows after `retry_in`.
    Disconnected {
        reason: Disconnect,
        retry_in: Duration,
    },
    /// The client was closed and will not reconnect.
    Closed,
}

impl ConnectionState {
    pub fn is_connected(&self) -> bool {
        matches!(self, Self::Connected { .. })
    }
}

/// Something the client observed, delivered in order.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Event {
    State(ConnectionState),
    /// A control message from the server. `Welcome` is reported as [`ConnectionState::Connected`] instead.
    Message(ServerMessage),
    /// An utterance failed in the client and will produce no transcript. See [`crate::Sender::send`].
    UtteranceFailed(FailedUtterance),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describes_disconnections() {
        let cases = [
            (
                Disconnect::Address("x".into()),
                "cannot resolve the server address: x",
            ),
            (Disconnect::Transport("y".into()), "connection failed: y"),
            (Disconnect::Status(404), "server responded with status 404"),
            (Disconnect::Protocol("z".into()), "protocol error: z"),
            (Disconnect::Timeout, "timed out waiting for the server"),
            (
                Disconnect::Certificate("ab".into()),
                "the server's certificate has changed (it presented ab)",
            ),
            (Disconnect::Closed, "server closed the connection"),
        ];
        for (reason, text) in cases {
            assert_eq!(reason.to_string(), text);
        }
        assert!(
            ConnectionState::Connected {
                source: 1,
                server: "s".into(),
                role: tomori_protocol::ConnectionRole::Owner,
                certificate: None,
                backends: vec![],
                roots: vec![],
                environments: vec![],
                utterance_limit: None,
            }
            .is_connected()
        );
        assert!(!ConnectionState::Closed.is_connected());
    }
}
