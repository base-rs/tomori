// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The identity of Tomori's Sky environment, a moonlit sky over a platform and a shrine: what its server and client halves share. It has no dependencies, and no renderer, so that a server offers Sky without building one; the renderer is `tomori-environment-sky-client`. See `design/environments.md`.
//!
//! ```
//! assert_eq!((tomori_environment_sky::ID, tomori_environment_sky::VERSION), ("sky", 1));
//! ```

/// The identifier spaces name Sky by.
pub const ID: &str = "sky";

/// Sky's protocol version, which a server offers it at and a client presents it at. Raise it, in this crate alone, when a client of the earlier version could not present Sky as the server now offers it.
pub const VERSION: u32 = 1;

/// The label clients show for Sky.
pub const LABEL: &str = "Sky";
