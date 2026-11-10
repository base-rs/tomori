// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The interface of an environment's server side.
//!
//! An environment may have a server side: a program in the Tomori server that runs while a space is bound to the environment, and that can reach whatever the server can. The server starts it with the [`Space`] it runs for, on a thread of its own, and cancels it when the space is bound to another environment, closes, or the server stops; see `design/environments.md`.
//!
//! [`sightlines`] holds the rule every environment's tests check of where it has participants arrive and puts the Tomori wall's home and the agent's: that the wall can be read, and the agent seen, from the arrival.
//!
//! Through its [`Space`], a side keeps state for the clients in the space that present the environment, sends them events, describes what they can point at for the space's agent, and takes the intents their participants send it. The state, events and intents are the environment's own JSON, which only its own crates read: the server carries them without reading them. An object it describes may carry a reference, which the agent can inspect for structured details that the side's [`Inspector`] gives.
//!
//! A side may also publish panels onto its environment's own walls, text, lists of [rows](Row) or [charts](chart), by ids it chooses, which every client draws as it draws the agent's panels: see [`Space::publish`].
//!
//! The server bounds what a side shares, announces, describes and answers by the limits this crate exports, which it enforces: [`STATE_LIMIT`], [`EVENT_LIMIT`], [`OBJECT_LIMIT`], [`IDENTITY_LIMIT`], [`DESCRIPTION_LIMIT`], [`REFERENCE_BYTES`] and [`INSPECTION_LIMIT`], and what it publishes by [`PANEL_ID_BYTES`], [`PANEL_LIMIT`], [`ROW_LIMIT`], [`ROW_ID_BYTES`] and [`ROW_CHARS`]. A state or event past its limit is refused, so a side keeps within them itself, and can test that it does.
use std::collections::VecDeque;
use std::fmt;
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::Duration;

use base_scheduler::Cancellation;

mod panel;
pub mod sightlines;

pub use panel::{
    PANEL_ID_BYTES, PANEL_LIMIT, Panel, ROW_CHARS, ROW_ID_BYTES, ROW_LIMIT, Row, valid_panel_id,
    valid_row_id,
};
/// A chart panel's native chart, as [`Panel::chart`] takes it.
pub use tomori_protocol::chart;

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
/// It may also have a reference, a short, stable name the space's agent can [inspect](Inspector) it by, such as
/// `lantern/3` or `panel/checkout-error-rate`, which the agent is told with the description, after the environment's
/// identifier, as `(id garden:lantern/3)`. See [`Object::with_reference`].
///
/// ```
/// use tomori_environment::Object;
///
/// let object = Object::new("lantern-3", "a lantern glowing by the gate");
/// assert_eq!(object.id(), "lantern-3");
/// assert_eq!(object.description(), "a lantern glowing by the gate");
/// assert_eq!(object.reference(), None);
/// let object = object.with_reference("lantern/3");
/// assert_eq!(object.reference(), Some("lantern/3"));
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Object {
    id: String,
    description: String,
    reference: Option<String>,
}

/// The longest reference of an object, in bytes: a longer one is left out, and the object described without it.
pub const REFERENCE_BYTES: usize = 64;

/// The largest state a side shares with [`Space::share`] or [`Space::announce`], as JSON text, in bytes: 64 KiB. The
/// server refuses a larger one, saying why. The server's tests check that it equals `tomori_protocol::STATE_LIMIT`,
/// the largest state the protocol carries.
pub const STATE_LIMIT: usize = 64 * 1024;

/// The largest event a side sends with [`Space::announce`], as JSON text, in bytes: 4 KiB. The server refuses a larger
/// one, saying why. The server's tests check that it equals `tomori_protocol::MESSAGE_LIMIT`, the largest event the
/// protocol carries.
pub const EVENT_LIMIT: usize = 4 * 1024;

/// The most objects a side describes with [`Space::describe`] that the server keeps: those after are left out.
pub const OBJECT_LIMIT: usize = 32;

/// The longest identity of an object, in bytes: the server leaves out an object whose identity is longer, or empty.
pub const IDENTITY_LIMIT: usize = 128;

/// The longest description of an object told to the space's agent, in characters, once the server has cleaned it onto
/// one line: a longer one is cut to this many, followed by the `…` that marks it cut.
pub const DESCRIPTION_LIMIT: usize = 200;

/// The largest answer of an [`Inspector`], as JSON text, in bytes: 16 KiB. The server refuses a larger one, and tells
/// the agent so. Within it, the server cuts each string to 2,000 characters, and the message of an [`Error`] the
/// inspector answers with to 512; those are not exported, since the server cuts what is past them rather than refusing
/// it.
pub const INSPECTION_LIMIT: usize = 16 * 1024;

/// Whether `reference` is one the server takes: 1 to [`REFERENCE_BYTES`] bytes of ASCII letters, digits, `-`, `_`,
/// `.` and `/`, so that it can never end the agent's context block or be read as anything but a name.
///
/// ```
/// use tomori_environment::valid_reference;
///
/// assert!(valid_reference("hen/marigold"));
/// assert!(valid_reference("panel/checkout-error-rate.p99"));
/// assert!(!valid_reference(""));
/// assert!(!valid_reference("hen marigold"));
/// assert!(!valid_reference("farmyard:hen"));
/// assert!(!valid_reference("hen]"));
/// ```
pub fn valid_reference(reference: &str) -> bool {
    (1..=REFERENCE_BYTES).contains(&reference.len())
        && reference
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'/'))
}

impl Object {
    /// The object `id`, described as `description`.
    pub fn new(id: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            description: description.into(),
            reference: None,
        }
    }

    /// This object, which the space's agent can inspect by `reference`, unique among the objects the side describes
    /// and the same for as long as the object is: the side's [`Inspector`] is asked about it by this reference. One
    /// that [`valid_reference`] refuses is left out by the server, which describes the object without it.
    pub fn with_reference(mut self, reference: impl Into<String>) -> Self {
        self.reference = Some(reference.into());
        self
    }

    /// The identity clients name it by.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// What the space's agent is told of it.
    pub fn description(&self) -> &str {
        &self.description
    }

    /// The reference the space's agent can inspect it by, if it has one.
    pub fn reference(&self) -> Option<&str> {
        self.reference.as_deref()
    }
}

/// How a server side answers the space's agent when it inspects something the side described with a reference, such
/// as what a participant points at. See `design/environments.md`.
///
/// The server calls it on a thread of its own, never the side's, so that a side busy with its own work, such as a slow
/// query, does not hold up the agent, and it may be called while the side runs; it is to answer from what the side
/// shares with it, such as a snapshot behind a lock, or with a query of its own. The server waits for an answer for at
/// most a few seconds, and refuses one that is not JSON or is larger than [`INSPECTION_LIMIT`], 16 KiB.
///
/// What it answers is told to the space's agent, which reads it as data, never as instructions: the server removes
/// control and format characters from every string and cuts long ones. It must hold nothing that should stay in the
/// server, such as an endpoint, a credential or a token, since the agent may repeat or project it; an environment that
/// reads a data source answers with what it asked and what it read, never how it reached the source.
///
/// A closure that takes a reference and answers as [`Inspector::inspect`] does is an inspector:
///
/// ```
/// use tomori_environment::{Inspector, Result};
///
/// let lanterns = |reference: &str| -> Result<Option<String>> {
///     Ok((reference == "lantern/3").then(|| r#"{"lit":true}"#.to_owned()))
/// };
/// assert_eq!(lanterns.inspect("lantern/3"), Ok(Some(r#"{"lit":true}"#.to_owned())));
/// assert_eq!(lanterns.inspect("lantern/9"), Ok(None));
/// ```
pub trait Inspector: Send + Sync {
    /// The details of what the side described as `reference`, as JSON text: whatever the environment chooses to say,
    /// such as a hen's activity and where she is, or a metric's query and the range it shows. `Ok(None)` when the side
    /// has nothing by that reference, as when what it named has gone; an [`Error`], whose message the agent is told,
    /// when it could not answer.
    fn inspect(&self, reference: &str) -> Result<Option<String>>;
}

impl<F> Inspector for F
where
    F: Fn(&str) -> Result<Option<String>> + Send + Sync,
{
    fn inspect(&self, reference: &str) -> Result<Option<String>> {
        self(reference)
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
    /// Publish `panel` onto one of the environment's own walls, or update the panel the side published with its id in
    /// place. An outlet that takes no panels, as one made before panels were, keeps nothing.
    fn publish(&self, panel: Panel) -> Result<()> {
        let _ = panel;
        Ok(())
    }
    /// Take away the panel the side published as `id`, if it is there.
    fn retract(&self, id: &str) -> Result<()> {
        let _ = id;
        Ok(())
    }
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

/// The intents waiting for a side, which the server hands on and the side takes, in order, and the side's inspector,
/// which the server asks.
#[derive(Default)]
struct Waiting {
    intents: Mutex<VecDeque<Intent>>,
    ready: Condvar,
    inspector: Mutex<Option<Arc<dyn Inspector>>>,
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

    /// The inspector the side last gave with [`Space::inspect_with`], if it gave one.
    pub fn inspector(&self) -> Option<Arc<dyn Inspector>> {
        self.0
            .inspector
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
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
    /// not JSON or is larger than [`STATE_LIMIT`], 64 KiB, saying why.
    pub fn share(&self, state: impl Into<String>) -> Result<()> {
        self.outlet.share(state.into())
    }

    /// Send `event`, the environment's own JSON text, to everyone in the space now who presents the environment at its
    /// version, and keep `state`, which already includes the event, for whoever joins later, at once, so that nobody
    /// misses the event or sees it twice. The server refuses an event larger than [`EVENT_LIMIT`], 4 KiB, or a state
    /// larger than [`STATE_LIMIT`], saying why.
    pub fn announce(&self, event: impl Into<String>, state: impl Into<String>) -> Result<()> {
        self.outlet.announce(event.into(), state.into())
    }

    /// Describe the objects a participant can point at in the space, replacing those described before, so that the
    /// server can tell the space's agent what a participant points at as they speak. The server keeps at most
    /// [`OBJECT_LIMIT`], 32, each identity at most [`IDENTITY_LIMIT`], 128 bytes, and each description cleaned and cut
    /// to [`DESCRIPTION_LIMIT`], 200 characters, with its reference, if it has one that [`valid_reference`] takes, of
    /// at most [`REFERENCE_BYTES`], which the agent may inspect it by (see [`Space::inspect_with`]).
    pub fn describe(&self, objects: Vec<Object>) -> Result<()> {
        self.outlet.describe(objects)
    }

    /// Publish `panel` onto the environment's own wall it names, for everyone in the space, as the agent's panels are
    /// shown: or, where the side published a panel with its id before, update that panel in place, replacing what it
    /// shows and keeping where it is, wherever a participant or the agent moved it. Participants and the agent move,
    /// point at and dismiss it as any panel; one dismissed stays away, its updates ignored, until the side
    /// [retracts](Space::retract) it and publishes it again, the space comes back to the environment or the server restarts.
    ///
    /// The server refuses, saying why: an id [`valid_panel_id`] refuses; a wall the environment does not declare, or a
    /// slot it does not have, or a full wall; a chart [`chart::Chart::cleaned`] refuses; more than [`PANEL_LIMIT`], 32,
    /// panels in the space; a list panel of more than [`ROW_LIMIT`] rows, a row id [`valid_row_id`] refuses or that
    /// another of its rows has, or a highlight that names none of its rows or is not a list panel's;
    /// and more than 80 calls a second, after a burst of 160. It makes a panel's changes at most about once a second, and
    /// 16 changes, or 128 KiB of panels, a second in the space, after a burst of 32 or 512 KiB, counting only what changes:
    /// a side may publish more often, and an update that comes too soon waits, the newest replacing the one before, and
    /// is made as soon as it may be, so that the latest always arrives. One that waits is refused at once for what can
    /// be known then, its wall, its slot and the 32, counting those waiting; one refused once it may be made, as its wall
    /// filled meanwhile, is dropped, which the server logs. When more than 12 panels float in the space, the one floating
    /// longest goes, whoever's it is, so that the agent's panels can push away one of the side's a participant left
    /// floating, which then stays away as a dismissed one does. Once this side's `run` has returned, or it has
    /// failed, the server refuses every call, from any thread it left, and what it published rests until it runs again. It cleans and bounds what it keeps, as it does the agent's panels:
    /// a title of 80 characters on one line, a body of 30 lines and 1,200 characters. A list panel's body is a list
    /// item a row, which it writes, each row's text cleaned onto one line and cut after [`ROW_CHARS`], followed by `…`, a leading checkbox written with parentheses so that no row is a checklist item, and the row it
    /// highlights with a pointer before it; rows past the body's bound are left out whole.
    ///
    /// ```
    /// use tomori_environment::{Panel, Space};
    ///
    /// let space = Space::new("space-1-1", "Fleet", "/");
    /// space.publish(Panel::text("rota", "Rota", "- [ ] Feed the hens")).unwrap();
    /// space.retract("rota").unwrap();
    /// ```
    pub fn publish(&self, panel: Panel) -> Result<()> {
        self.outlet.publish(panel)
    }

    /// Take away the panel the side published as `id`, wherever it is now, for everyone in the space; a panel not there
    /// changes nothing.
    pub fn retract(&self, id: &str) -> Result<()> {
        self.outlet.retract(id)
    }

    /// Answer the space's agent with `inspector` when it inspects what this side described with a reference, replacing
    /// any inspector given before. A side that gives none has nothing more to say of what it describes. It answers for
    /// as long as the side's binding lasts, and the server forgets it with the binding.
    ///
    /// ```
    /// use tomori_environment::{Result, Space};
    ///
    /// let space = Space::new("space-1-1", "Fleet", "/");
    /// assert!(space.inbox().inspector().is_none());
    /// space.inspect_with(|reference: &str| -> Result<Option<String>> {
    ///     Ok(Some(format!("{{\"reference\":\"{reference}\"}}")))
    /// });
    /// let inspector = space.inbox().inspector().unwrap();
    /// assert_eq!(inspector.inspect("hen/pip").unwrap().unwrap(), r#"{"reference":"hen/pip"}"#);
    /// ```
    pub fn inspect_with(&self, inspector: impl Inspector + 'static) {
        *self
            .waiting
            .inspector
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(Arc::new(inspector));
    }

    /// Where the server hands on intents for this side, and finds its inspector.
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
        assert_eq!(space.publish(Panel::text("a", "b", "c")), Ok(()));
        assert_eq!(space.retract("a"), Ok(()));
    }

    #[test]
    fn an_outlet_made_before_panels_keeps_none() {
        struct Older;
        impl Outlet for Older {
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
        let space = Space::new("space-1-2", "Fleet", "/").with_outlet(Arc::new(Older));
        assert_eq!(space.share("{}"), Ok(()));
        assert_eq!(space.announce("{}", "{}"), Ok(()));
        assert_eq!(space.describe(Vec::new()), Ok(()));
        assert_eq!(space.publish(Panel::text("a", "b", "c")), Ok(()));
        assert_eq!(space.retract("a"), Ok(()));
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
    fn an_object_may_have_a_reference_of_safe_characters() {
        let object = Object::new("chicken-2", "Marigold").with_reference("hen/marigold");
        assert_eq!(object.reference(), Some("hen/marigold"));
        assert_eq!(Object::new("a", "b").reference(), None);
        assert!(valid_reference("a"));
        assert!(valid_reference(&"a".repeat(REFERENCE_BYTES)));
        assert!(!valid_reference(&"a".repeat(REFERENCE_BYTES + 1)));
        for refused in ["", "a b", "a:b", "a[b", "a]b", "a\"b", "a\nb", "ä"] {
            assert!(!valid_reference(refused), "{refused:?}");
        }
    }

    /// The limits a side keeps within are those `design/environments.md` states, so that changing one is deliberate.
    #[test]
    fn the_limits_a_side_keeps_within_are_those_the_design_states() {
        assert_eq!(STATE_LIMIT, 64 * 1024);
        assert_eq!(EVENT_LIMIT, 4 * 1024);
        assert_eq!(OBJECT_LIMIT, 32);
        assert_eq!(IDENTITY_LIMIT, 128);
        assert_eq!(DESCRIPTION_LIMIT, 200);
        assert_eq!(REFERENCE_BYTES, 64);
        assert_eq!(INSPECTION_LIMIT, 16 * 1024);
    }

    #[test]
    fn a_side_s_inspector_is_found_through_its_inbox_and_replaced() {
        let space = Space::new("space-1-2", "Fleet", "/srv/fleet");
        let inbox = space.inbox();
        assert!(inbox.inspector().is_none());
        space.inspect_with(|_: &str| -> Result<Option<String>> { Ok(None) });
        assert_eq!(inbox.inspector().unwrap().inspect("x"), Ok(None));
        space.inspect_with(|_: &str| -> Result<Option<String>> { Err("no route".into()) });
        assert_eq!(
            inbox.inspector().unwrap().inspect("x"),
            Err(Error::new("no route"))
        );
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
