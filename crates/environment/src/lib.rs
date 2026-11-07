// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The interface of an environment's server side.
//!
//! An environment may have a server side: a program in the Tomori server that runs while a space is bound to the environment, and that can reach whatever the server can. The server starts it with the [`Space`] it runs for, on a thread of its own, and cancels it when the space is bound to another environment, closes, or the server stops; see `design/environments.md`.
//!
//! Through its [`Space`], a side keeps state for the clients in the space that present the environment, sends them events, describes what they can point at for the space's agent, and takes the intents their participants send it. The state, events and intents are the environment's own JSON, which only its own crates read: the server carries them without reading them.
use std::collections::VecDeque;
use std::fmt;
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::Duration;

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

/// Something a participant asked of the environment in a space, as their client sent it: who asked, and the
/// environment's own intent, as JSON text, which the side validates before acting on it. Only participants ask; the
/// server has already checked that they are one, in the space, and that the intent is within its limits and not too
/// soon after their last. See `design/environments.md`.
///
/// ```
/// use tomori_environment::Intent;
///
/// let intent = Intent::new(3, r#"{"notice":1}"#);
/// assert_eq!(intent.participant(), 3);
/// assert_eq!(intent.payload(), r#"{"notice":1}"#);
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Intent {
    participant: u32,
    payload: String,
}

impl Intent {
    /// The intent `payload` of the participant numbered `participant`.
    pub fn new(participant: u32, payload: impl Into<String>) -> Self {
        Self {
            participant,
            payload: payload.into(),
        }
    }

    /// Who asked: the number the server gave the participant, which everyone in the space knows them by, as the
    /// `source` of their audio and presence. The same participant has the same number for as long as the server keeps
    /// them.
    pub fn participant(&self) -> u32 {
        self.participant
    }

    /// The environment's own intent, as JSON text.
    pub fn payload(&self) -> &str {
        &self.payload
    }
}

/// Something the environment presents in a space that a participant can point at, as the side describes it: the
/// identity clients name it by, and what the space's agent is told of it, such as "a lantern glowing by the gate".
/// The server cleans the description onto one line and bounds it, as it does every description it gives the agent.
///
/// ```
/// use tomori_environment::Object;
///
/// let object = Object::new("lantern-3", "a lantern glowing by the gate");
/// assert_eq!(object.id(), "lantern-3");
/// assert_eq!(object.description(), "a lantern glowing by the gate");
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Object {
    id: String,
    description: String,
}

impl Object {
    /// The object `id`, described as `description`.
    pub fn new(id: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            description: description.into(),
        }
    }

    /// The identity clients name it by.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// What the space's agent is told of it.
    pub fn description(&self) -> &str {
        &self.description
    }
}

/// Where a server side's state, events and descriptions go: the server, which sends them to the clients in the space
/// that present the environment, or nowhere, for a space made without one, as in a side's own tests. See
/// [`Space::with_outlet`].
pub trait Outlet: Send + Sync {
    /// Keep `state` for the space, replacing what was kept, and send it to everyone in the space now and to whoever joins
    /// later.
    fn share(&self, state: String) -> Result<()>;
    /// Send `event` to everyone in the space now, and keep `state`, which already includes it, for whoever joins later.
    fn announce(&self, event: String, state: String) -> Result<()>;
    /// Describe the objects a participant can point at in the space, replacing those described before.
    fn describe(&self, objects: Vec<Object>) -> Result<()>;
}

/// An outlet that keeps nothing and sends nothing, for a space made without one.
struct Nowhere;

impl Outlet for Nowhere {
    fn share(&self, _state: String) -> Result<()> {
        Ok(())
    }

    fn announce(&self, _event: String, _state: String) -> Result<()> {
        Ok(())
    }

    fn describe(&self, _objects: Vec<Object>) -> Result<()> {
        Ok(())
    }
}

/// The most intents waiting for a side: those beyond it are dropped, as the side is not keeping up.
pub const WAITING_INTENT_LIMIT: usize = 64;

/// The intents waiting for a side, which the server hands on and the side takes, in order.
#[derive(Default)]
struct Waiting {
    intents: Mutex<VecDeque<Intent>>,
    ready: Condvar,
}

/// Where the server hands on the intents participants send to the side of a space: see [`Space::inbox`].
#[derive(Clone)]
pub struct Inbox(Arc<Waiting>);

impl Inbox {
    /// Hand on `intent`, unless [`WAITING_INTENT_LIMIT`] are already waiting, when it is dropped. Says whether it was handed
    /// on.
    pub fn push(&self, intent: Intent) -> bool {
        let mut intents = self
            .0
            .intents
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if intents.len() >= WAITING_INTENT_LIMIT {
            return false;
        }
        intents.push_back(intent);
        self.0.ready.notify_all();
        true
    }
}

/// The space a server side runs for: what it is told about the space, when to stop, and its channel to the clients in
/// the space: the state it keeps for them, its events, the objects it describes, and the intents their participants
/// send it.
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
    outlet: Arc<dyn Outlet>,
    waiting: Arc<Waiting>,
}

impl Space {
    /// The space `id`, titled `title`, running in `directory`, with a cancellation of its own, whose state and events go
    /// nowhere.
    pub fn new(
        id: impl Into<String>,
        title: impl Into<String>,
        directory: impl Into<String>,
    ) -> Self {
        Self::with_cancellation(id, title, directory, Cancellation::new())
    }

    /// The space `id`, titled `title`, running in `directory`, whose server side stops when `cancellation` is cancelled,
    /// and whose state and events go nowhere.
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
            outlet: Arc::new(Nowhere),
            waiting: Arc::default(),
        }
    }

    /// This space, whose state, events and descriptions go to `outlet`, as the server's do, or a test's.
    ///
    /// ```
    /// use std::sync::{Arc, Mutex};
    /// use tomori_environment::{Object, Outlet, Result, Space};
    ///
    /// #[derive(Default)]
    /// struct Kept(Mutex<Vec<String>>);
    ///
    /// impl Outlet for Kept {
    ///     fn share(&self, state: String) -> Result<()> {
    ///         self.0.lock().unwrap().push(state);
    ///         Ok(())
    ///     }
    ///     fn announce(&self, event: String, state: String) -> Result<()> {
    ///         self.0.lock().unwrap().extend([event, state]);
    ///         Ok(())
    ///     }
    ///     fn describe(&self, _objects: Vec<Object>) -> Result<()> {
    ///         Ok(())
    ///     }
    /// }
    ///
    /// let kept = Arc::new(Kept::default());
    /// let space = Space::new("space-1-1", "Fleet", "/").with_outlet(kept.clone());
    /// space.share("{}").unwrap();
    /// assert_eq!(*kept.0.lock().unwrap(), ["{}"]);
    /// ```
    pub fn with_outlet(mut self, outlet: Arc<dyn Outlet>) -> Self {
        self.outlet = outlet;
        self
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

    /// Keep `state`, the environment's own JSON text, for the space, replacing what was kept, and send it to everyone in
    /// the space who presents the environment at its version, now and as they join. The server refuses text that is
    /// not JSON or is larger than its limit, 64 KiB, saying why.
    pub fn share(&self, state: impl Into<String>) -> Result<()> {
        self.outlet.share(state.into())
    }

    /// Send `event`, the environment's own JSON text, to everyone in the space now who presents the environment at its
    /// version, and keep `state`, which already includes the event, for whoever joins later, at once, so that nobody
    /// misses the event or sees it twice. The server refuses an event larger than 4 KiB, saying why.
    pub fn announce(&self, event: impl Into<String>, state: impl Into<String>) -> Result<()> {
        self.outlet.announce(event.into(), state.into())
    }

    /// Describe the objects a participant can point at in the space, replacing those described before, so that the
    /// server can tell the space's agent what a participant points at as they speak. The server keeps at most 32, each
    /// identity at most 128 bytes, and each description cleaned and cut to 200 characters.
    pub fn describe(&self, objects: Vec<Object>) -> Result<()> {
        self.outlet.describe(objects)
    }

    /// Where the server hands on intents for this side.
    pub fn inbox(&self) -> Inbox {
        Inbox(self.waiting.clone())
    }

    /// The next intent a participant sent, waiting for one for at most `timeout`; `None` if none came. A side that also
    /// keeps time, or stops when cancelled, waits a little at a time.
    pub fn next_intent(&self, timeout: Duration) -> Option<Intent> {
        let intents = self
            .waiting
            .intents
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let (mut intents, _) = self
            .waiting
            .ready
            .wait_timeout_while(intents, timeout, |intents| intents.is_empty())
            .unwrap_or_else(PoisonError::into_inner);
        intents.pop_front()
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
    fn a_space_made_without_an_outlet_sends_nowhere() {
        let space = Space::new("space-1-2", "Fleet", "/srv/fleet");
        assert_eq!(space.share("{}"), Ok(()));
        assert_eq!(space.announce("{}", "{}"), Ok(()));
        assert_eq!(space.describe(vec![Object::new("a", "b")]), Ok(()));
    }

    #[test]
    fn intents_wait_in_order_and_beyond_the_limit_are_dropped() {
        let space = Space::new("space-1-2", "Fleet", "/srv/fleet");
        assert_eq!(space.next_intent(Duration::from_millis(1)), None);
        let inbox = space.inbox();
        for participant in 0..WAITING_INTENT_LIMIT as u32 {
            assert!(inbox.push(Intent::new(participant, "{}")));
        }
        assert!(!inbox.push(Intent::new(99, "{}")));
        assert_eq!(
            space
                .next_intent(Duration::ZERO)
                .map(|intent| intent.participant()),
            Some(0)
        );
        // One taken, another can wait.
        assert!(inbox.push(Intent::new(100, "{}")));
        // Pushed from another thread, a waiting side takes it.
        let space = Space::new("space-1-3", "Fleet", "/");
        let inbox = space.inbox();
        let pusher = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            inbox.push(Intent::new(7, "[]"))
        });
        assert_eq!(
            space.next_intent(Duration::from_secs(10)),
            Some(Intent::new(7, "[]"))
        );
        assert!(pusher.join().unwrap());
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
