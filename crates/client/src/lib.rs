// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The Tomori client: one long-lived duplex connection to the server, with automatic reconnection, plus real-time playback of received audio.
//!
//! [`Client`] is the connection itself. [`connection::Connection`] builds a whole client's connection on it for a [`connection::Session`], the client's conversation with the server: it chooses the server, keeps its certificate and token for this device, records from the microphone, plays through the speaker and cues what happens.
//!
//! See `design/server.md`, `design/live-audio.md` and `design/clients.md`.
pub mod capture;
pub mod connection;
pub mod engagement;
pub mod feedback;
pub mod identity;
pub mod playback;
pub mod server_choice;
pub mod speaker;

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
mod system;
#[cfg(test)]
mod test_support;

pub use backoff::Backoff;
pub use client::{Client, Sender};
pub use connection_state::{ConnectionState, Disconnect, Event};
pub use events::Events;
pub use microphone::Microphone;
pub use options::{Options, Transport};
pub use outbox::{FailedUtterance, Membership, SendError, UtteranceFailure};
pub use tomori_protocol::Keepalive;
pub use tomori_security::Fingerprint;

/// The shortest frame whose arrival is reported as [`Event::Receiving`], as it arrives: a quarter of
/// [`tomori_protocol::MAX_FRAME_BYTES`], which takes long enough to arrive over a slow link to be worth showing.
pub const PROGRESS_BYTES: usize = tomori_protocol::MAX_FRAME_BYTES / 4;
