// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The interface of an environment's server side.
//!
//! An environment may have a server side: a program in the Tomori server that runs while a space is bound to the environment, and that can reach whatever the server can. The server starts it with the [`Space`] it runs for, on a thread of its own, and cancels it when the space is bound to another environment, closes, or the server stops; see `design/environments.md`.
use std::fmt;

use base_scheduler::Cancellation;

/// Why a server side stopped before its space was done with it, as the space shows it.
///
/// Its message is shown to everyone in the space, visitors included, so it must hold nothing that should stay in the server, such as an endpoint or a credential. A `String` or `&str` converts into one, so `?` works on a `Result` whose error is already a message for the space.
///
/// ```
/// use tomori_environment::Error;
///
/// let error = Error::new("The fleet is unreachable.");
/// assert_eq!(error.message(), "The fleet is unreachable.");
/// assert_eq!(error.to_string(), "The fleet is unreachable.");
/// assert_eq!(Error::from("The fleet is unreachable."), error);
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Error {
    message: String,
}

impl Error {
    /// The error shown in the space as `message`.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// What the space shows.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

impl From<String> for Error {
    fn from(message: String) -> Self {
        Self::new(message)
    }
}

impl From<&str> for Error {
    fn from(message: &str) -> Self {
        Self::new(message)
    }
}

/// What a server side returns: `Ok` once its space's cancellation is cancelled, or the [`Error`] its space shows.
pub type Result<T> = std::result::Result<T, Error>;

/// The space a server side runs for: what it is told about the space, and when to stop.
///
/// ```
/// use tomori_environment::Space;
///
/// let space = Space::new("space-1-1", "Fleet", "/home/ada/fleet");
/// assert_eq!(space.id(), "space-1-1");
/// assert_eq!(space.title(), "Fleet");
/// assert_eq!(space.directory(), "/home/ada/fleet");
/// assert!(!space.cancellation().is_cancelled());
/// ```
#[non_exhaustive]
pub struct Space {
    id: String,
    title: String,
    directory: String,
    cancellation: Cancellation,
}

impl Space {
    /// The space `id`, titled `title`, running in `directory`, with a cancellation of its own.
    pub fn new(
        id: impl Into<String>,
        title: impl Into<String>,
        directory: impl Into<String>,
    ) -> Self {
        Self::with_cancellation(id, title, directory, Cancellation::new())
    }

    /// The space `id`, titled `title`, running in `directory`, whose server side stops when `cancellation` is cancelled.
    pub fn with_cancellation(
        id: impl Into<String>,
        title: impl Into<String>,
        directory: impl Into<String>,
        cancellation: Cancellation,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            directory: directory.into(),
            cancellation,
        }
    }

    /// The space's identifier, which is the same for everyone in it and across the server's restarts.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The space's title when the side was started.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// The canonical directory the space's agent runs in.
    pub fn directory(&self) -> &str {
        &self.directory
    }

    /// Cancelled when the side is to stop: when the space is bound to another environment, closes, or the server stops. A side that ignores it is abandoned, and the server stops regardless. A side with nothing else to wait for can block on [`Cancellation::cancelled`] with any executor rather than poll it.
    pub fn cancellation(&self) -> &Cancellation {
        &self.cancellation
    }
}

/// An environment's server side.
///
/// The server calls [`ServerSide::run`] once for each time a space is bound to the environment, on a thread of its own, so it may block, and neither it nor a panic in it affects the hub, other spaces or the conversation. It runs until its space's cancellation is cancelled, and returns `Ok` then; an [`Error`], or a panic, is shown in the space, which keeps its environment and its conversation, and the side is not run again for that binding.
///
/// The error is shown to everyone in the space, visitors included, so it must hold nothing that should stay in the server, such as an endpoint or a credential. A panic is contained only where panics unwind: in a build with `panic = "abort"` it ends the server.
///
/// ```
/// use tomori_environment::{Result, ServerSide, Space};
///
/// struct Quiet;
///
/// impl ServerSide for Quiet {
///     fn run(&self, space: &Space) -> Result<()> {
///         while !space.cancellation().is_cancelled() {
///             std::thread::sleep(std::time::Duration::from_millis(10));
///         }
///         Ok(())
///     }
/// }
///
/// let space = Space::new("space-1-1", "Fleet", "/");
/// space.cancellation().cancel();
/// assert_eq!(Quiet.run(&space), Ok(()));
/// ```
pub trait ServerSide: Send + Sync {
    /// Run for `space`, returning when its cancellation is cancelled, or with the error to show in the space.
    fn run(&self, space: &Space) -> Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_error_is_its_message() {
        let error = Error::from("The fleet is unreachable.".to_owned());
        assert_eq!(error, Error::new("The fleet is unreachable."));
        assert_eq!(format!("{error}"), "The fleet is unreachable.");
        let source: &dyn std::error::Error = &error;
        assert!(source.source().is_none());
    }

    #[test]
    fn a_space_is_what_it_was_given() {
        let space = Space::new("space-1-2", "Fleet", "/srv/fleet");
        assert_eq!(space.id(), "space-1-2");
        assert_eq!(space.title(), "Fleet");
        assert_eq!(space.directory(), "/srv/fleet");
    }

    #[test]
    fn a_space_stops_with_the_cancellation_it_was_given() {
        let cancellation = Cancellation::new();
        let space = Space::with_cancellation("id", "title", "/", cancellation.child());
        assert!(!space.cancellation().is_cancelled());
        cancellation.cancel();
        assert!(space.cancellation().is_cancelled());
    }
}
