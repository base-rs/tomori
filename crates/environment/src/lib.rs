// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The interface of an environment's server side.
//!
//! An environment may have a server side: a program in the Tomori server that runs while a space is bound to the environment, and that can reach whatever the server can. The server starts it with the [`Space`] it runs for, on a thread of its own, and cancels it when the space is bound to another environment, closes, or the server stops; see `design/environments.md`.
use base_scheduler::Cancellation;

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

    /// Cancelled when the side is to stop: when the space is bound to another environment, closes, or the server stops. A side that ignores it is abandoned, and the server stops regardless.
    pub fn cancellation(&self) -> &Cancellation {
        &self.cancellation
    }
}

/// An environment's server side.
///
/// The server calls [`ServerSide::run`] once for each time a space is bound to the environment, on a thread of its own, so it may block, and neither it nor a panic in it affects the hub, other spaces or the conversation. It runs until its space's cancellation is cancelled, and returns `Ok` then; an error, or a panic, is shown in the space, which keeps its environment and its conversation, and the side is not run again for that binding.
///
/// The error is shown to everyone in the space, visitors included, so it must hold nothing that should stay in the server, such as an endpoint or a credential. A panic is contained only where panics unwind: in a build with `panic = "abort"` it ends the server.
///
/// ```
/// use tomori_environment::{ServerSide, Space};
///
/// struct Quiet;
///
/// impl ServerSide for Quiet {
///     fn run(&self, space: &Space) -> Result<(), String> {
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
    fn run(&self, space: &Space) -> Result<(), String>;
}

#[cfg(test)]
mod tests {
    use super::*;

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
