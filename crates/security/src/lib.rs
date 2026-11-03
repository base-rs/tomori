// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Authentication, tokens, the server's certificate and certificate pinning, shared by the Tomori server and its clients. See `design/security.md`.
//!
//! Authenticators say who a connection is, never what it may do: the server keeps roles against the [`Principal`]s they report. The `issue` feature adds the server's side: the [`TokenStore`], which issues tokens and authenticates them, and the server's certificate.

mod authenticate;
#[cfg(feature = "issue")]
mod certificate;
mod fingerprint;
mod hex;
pub mod pairing;
mod pin;
#[cfg(feature = "issue")]
mod store;
mod token;

pub use authenticate::{
    Authenticator, Chain, Credentials, Outcome, Principal, TOKEN_COOKIE, Verdict,
};
#[cfg(feature = "issue")]
pub use certificate::ServerCertificate;
pub use fingerprint::{Fingerprint, FingerprintError};
pub use pin::{Pinned, client_config};
#[cfg(feature = "issue")]
pub use store::{Entry, Grant, StoreError, TokenStore, create_private_directory, write_private};
pub use token::{TOKEN_PREFIX, generate as generate_token};
