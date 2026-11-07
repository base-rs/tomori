// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The identity of Tomori's GitHub environment, a prototype whose spaces show one repository's open issues and pull requests as cards to lay out: what its server and client halves share. It has no dependencies, so that a client builds it without the server's GitHub client, and the server without a renderer. The server half, which holds the token, is `tomori-environment-github-server`, and the client half `tomori-environment-github-client`. See `design/github.md` and `design/environments.md`.
//!
//! ```
//! assert_eq!((tomori_environment_github::ID, tomori_environment_github::VERSION), ("github", 1));
//! ```

/// The identifier spaces name the GitHub environment by.
pub const ID: &str = "github";

/// The GitHub environment's protocol version, which a server offers it at and a client presents it at. Raise it, in this crate alone, when a client of the earlier version could not present it as the server now offers it.
pub const VERSION: u32 = 1;

/// The label clients show for the GitHub environment.
pub const LABEL: &str = "GitHub";
