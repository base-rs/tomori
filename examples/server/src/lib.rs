// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! An example of a server built on `tomori-server` by another project: it offers a backend and an environment of its own, registered through [`tomori_server::Builder`], beside Tomori's, without changing Tomori. Its executable, `tomori-example-server`, takes the same command line as `tomori-server`, so `tomori-example-server --backend placeholder` offers the placeholder backend.

mod placeholder;

pub use placeholder::{Placeholder, PlaceholderSession};

use tomori_server::{Builder, Environment, Server};

/// The placeholder environment's identifier. A client shows the Minimal grid for it and names it, since no client has its client half.
pub const ENVIRONMENT: &str = "placeholder";

/// What the placeholder environment tells a space's agent, its layer of the space's instructions.
pub const INSTRUCTIONS: &str =
    "This space is in the placeholder environment, which presents nothing yet.";

/// The placeholder environment.
pub fn environment() -> Environment {
    Environment::new(ENVIRONMENT, "Placeholder").with_instructions(INSTRUCTIONS)
}

/// Tomori's own server, with the placeholder backend and environment too.
pub fn builder() -> Builder {
    Server::builder()
        .backend(placeholder::NAME, || Ok(Placeholder))
        .environment(environment())
}
