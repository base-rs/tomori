// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The Tomori client: one long-lived duplex connection to the server, with automatic reconnection, plus real-time playback of received audio.
//!
//! See `design/server.md` and `design/live-audio.md`.

pub mod engagement;
pub mod playback;

/// Finding servers on the local network with Bonjour: [`tomori_discovery::browse`] yields servers to list, and [`tomori_discovery::Server::address`] gives the address for [`Options::new`]. On Android, hold a Wi-Fi multicast lock while browsing; see the [`tomori_discovery`] crate documentation.
pub use tomori_discovery as discovery;

mod backoff;
mod client;
mod connection_state;
mod events;
mod microphone;
mod options;
mod outbox;
mod resolve;
mod session;

pub use backoff::Backoff;
pub use client::{Client, Sender};
pub use connection_state::{ConnectionState, Disconnect, Event};
pub use events::Events;
pub use microphone::Microphone;
pub use options::{Options, Transport};
pub use outbox::{FailedUtterance, Membership, SendError, UtteranceFailure};
pub use tomori_protocol::Keepalive;
pub use tomori_security::Fingerprint;
