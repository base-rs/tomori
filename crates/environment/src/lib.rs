// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The interface of an environment's server side.
//!
//! An environment may have a server side: a program in the Tomori server that runs while a space is bound to the environment, and that can reach whatever the server can. The server starts it with the [`Space`] it runs for, on a thread of its own, and cancels it when the space is bound to another environment, closes, or the server stops; see `design/environments.md`.
//!
//! [`navmesh`] derives where participants may walk from the ground an environment draws, less what it declares solid, for its client's teleport profile and, later, for paths across it.
//!
//! [`sightlines`] holds the rule every environment's tests check of where it has participants arrive and puts the Tomori wall's home and the agent's: that the wall can be read, and the agent seen, from the arrival.
//!
//! Through its [`Space`], a side keeps state for the clients in the space that present the environment, sends them events, describes what they can point at for the space's agent, and takes the intents their participants send it. The state, events and intents are the environment's own JSON, which only its own crates read: the server carries them without reading them. An object it describes may carry a reference, which the agent can inspect for structured details that the side's [`Inspector`] gives.
//!
//! A side may place cluster anchors as its state changes, named places that workers, panels and the agent gather at, such as a rack that starts alerting, and remove them again: see [`Space::place_anchor`].
//!
//! A side may declare [actions](Action) on what it describes, which the space's agent takes when someone asks, each
//! reaching it as an [`Intent`] whose [`Intent::acted`] names it, which it answers with [`Space::answer`]: see
//! [`Space::declare_actions`].
//!
//! A side may also publish panels onto its environment's own walls, text, lists of [rows](Row) or [charts](chart), by ids it chooses, which every client draws as it draws the agent's panels: see [`Space::publish`]. A participant's choice of a row it marks [choosable](Row::choosable) reaches it as an [`Intent`] whose [`Intent::chosen`] names the row.
//!
//! The server bounds what a side shares, announces, describes and answers by the limits this crate exports, which it enforces: [`STATE_LIMIT`], [`EVENT_LIMIT`], [`OBJECT_LIMIT`], [`IDENTITY_LIMIT`], [`DESCRIPTION_LIMIT`], [`REFERENCE_BYTES`] and [`INSPECTION_LIMIT`], what it publishes by [`PANEL_ID_BYTES`], [`PANEL_LIMIT`], [`ROW_LIMIT`], [`ROW_ID_BYTES`] and [`ROW_CHARS`], and the cluster anchors it places by [`ANCHOR_LIMIT`], [`ANCHOR_NAME_BYTES`], [`ANCHOR_LABEL_LIMIT`], [`ANCHOR_INTERVAL`], [`ANCHOR_CHANGES_PER_SECOND`] and [`ANCHOR_BURST`]. A state or event past its limit is refused, so a side keeps within them itself, and can test that it does.
//!
//! A side learns where participants are in two ways: it may declare proximity [zones](Zone), whose occupancy the server
//! tells it as an [`Intent`] whose [`Intent::occupancy`] names the zone and how many are within it, see
//! [`Space::declare_zones`]; and it may read where each participant's head and hands are, and how engaged they are,
//! with [`Space::presence`]. Participants are named by their numbers alone, as everywhere else a side meets them.
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant};

use base_scheduler::Cancellation;

mod action;
mod context;
pub mod navmesh;
mod panel;
pub mod sightlines;
mod zone;

pub use action::{
    ACTION_DESCRIPTION_LIMIT, ACTION_LIMIT, ACTION_NAME_BYTES, ACTIONS_LIMIT, Acted, Action,
    CHOICE_LABEL_LIMIT, CHOICE_LIMIT, Choice, Kind, OFFER_LIMIT, OUTCOME_LIMIT, Outcome,
    PARAMETER_DESCRIPTION_LIMIT, PARAMETER_LIMIT, Parameter, TEXT_LIMIT, check_actions,
    valid_action_name,
};
pub use context::{
    Binding, CONTEXT_CHOICES, CONTEXT_ID_BYTES, CONTEXT_LABEL_CHARS, CONTEXT_PARAMETERS,
    CONTEXT_VALUE_CHARS, Context, ContextParameter, TIME, Time, valid_context_id,
};

pub use zone::{
    Around, Engagement, LEAST_ZONE_RADIUS, MOST_ZONE_DWELL, MOST_ZONE_HEIGHT, MOST_ZONE_MARGIN,
    MOST_ZONE_RADIUS, Occupancy, PRESENCE_LIMIT, PRESENCE_STALE, Shape, ZONE_INTERVAL, ZONE_LIMIT,
    ZONE_MARGIN, ZONE_NAME_BYTES, Zone, ZoneError, check_zones,
};

pub use panel::{
    CELL_CHARS, COLUMN_LIMIT, COLUMN_TITLE_CHARS, Column, FIELD_LIMIT, FIELD_NAME_BYTES,
    FIELD_VALUE_BYTES, FIELDS_BYTES, PANEL_ID_BYTES, PANEL_LIMIT, Panel, ROW_CHARS, ROW_ID_BYTES,
    ROW_LIMIT, Row, valid_field_name, valid_panel_id, valid_row_id,
};
/// How a list panel's column aligns its cells, as [`Column::aligned`] takes it.
pub use tomori_protocol::CellAlign;
/// A chart panel's native chart, as [`Panel::chart`] takes it.
pub use tomori_protocol::chart;
/// A cluster anchor, as [`Space::place_anchor`] takes it.
pub use tomori_protocol::cluster::Anchor;

/// The most cluster anchors in a space's binding, those its environment declares counted first: one placed beyond it
/// is refused.
pub const ANCHOR_LIMIT: usize = 64;

/// The longest name of an anchor, in bytes: as long as an object's reference.
pub const ANCHOR_NAME_BYTES: usize = REFERENCE_BYTES;

/// The longest label of an anchor told to the agent, in characters, once the server has cleaned it onto one line: a
/// longer one is cut to this many, followed by the `…` that marks it cut.
pub const ANCHOR_LABEL_LIMIT: usize = tomori_protocol::cluster::LABEL_CHARS;

/// How often the server makes one anchor's changes at most: a change that comes sooner waits.
pub const ANCHOR_INTERVAL: Duration = Duration::from_millis(900);

/// How many anchor changes the server makes in a space each second, after a burst of [`ANCHOR_BURST`].
pub const ANCHOR_CHANGES_PER_SECOND: u32 = 4;

/// How many anchor changes the server makes in a space at once before [`ANCHOR_CHANGES_PER_SECOND`] holds.
pub const ANCHOR_BURST: u32 = 8;

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
///
/// An intent may instead be a participant's choice of a row of a list panel the side published, which the server makes
/// itself, never from what a client sends as an intent: see [`Intent::chosen`]; or a change of where one of its panels
/// is, by a participant or the layout, or its dismissal: see [`Intent::moved`].
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Intent {
    participant: u32,
    payload: String,
    chosen: Option<Chosen>,
    acted: Option<Acted>,
    moved: Option<Moved>,
    occupancy: Option<Occupancy>,
    context: Option<Context>,
}

/// Where a panel the side published is now, as the server tells the side in a [`Moved`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PanelPlace {
    /// On one of the environment's own walls, by its place in the order the environment declares them, from 0, as
    /// [`Panel::on_wall`] names it, in the slot `slot`.
    Wall { wall: usize, slot: u32 },
    /// On a wall the environment does not declare, as one a participant added.
    OtherWall,
    /// Floating, on no wall.
    Floating,
    /// Thrown away: it stays away, its updates ignored, until the side [retracts](Space::retract) it and publishes it
    /// again.
    Dismissed,
}

/// A change of where a panel the side published is, as the server tells the side: the side's own id for the panel, as
/// it published it, and where it is now. The server tells every change of the wall a panel is on, whatever made it: a
/// participant's move, and the panel a dropped one swapped with; a wall removed or its grid shrunk, floating what it held;
/// the agent's moves; a panel put back, as the side publishes it again, somewhere other than the wall it asks for, as
/// where it rested before the side started again; and a dismissal, by a participant or by the layout, as the floating
/// bound giving one up. It never tells a move between the slots of one wall, a floating panel moved on, a retraction
/// or a run ending. See `design/panel-actions.md`.
///
/// ```
/// use tomori_environment::{Moved, PanelPlace};
///
/// let moved = Moved::new("system", PanelPlace::Floating);
/// assert_eq!((moved.panel(), moved.place()), ("system", PanelPlace::Floating));
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Moved {
    panel: String,
    place: PanelPlace,
}

impl Moved {
    /// The panel `panel`, by the side's id, now at `place`.
    pub fn new(panel: impl Into<String>, place: PanelPlace) -> Self {
        Self {
            panel: panel.into(),
            place,
        }
    }

    /// The side's id for the panel, as it published it.
    pub fn panel(&self) -> &str {
        &self.panel
    }

    /// Where it is now.
    pub fn place(&self) -> PanelPlace {
        self.place
    }
}

/// A participant's choice of a row of a list panel the side published, as the server hands it on: the side's own id
/// for the panel and the row's id, as it published them, and whether the participant chose it from the panel as it is
/// now. The server checked that the panel is this side's, shown in the space, that the row is on it now and marked
/// [choosable](crate::Row::choosable), and that the participant takes part in the space and is within the intents'
/// rate; it names the row by its id, never by where it stands in the list, so that a choice made just as the list
/// changed names the row the participant saw, or is refused, should that row have gone. See `design/panel-actions.md`.
///
/// ```
/// use tomori_environment::Chosen;
///
/// let chosen = Chosen::new("queue", "pod112", true);
/// assert_eq!((chosen.panel(), chosen.row(), chosen.current()), ("queue", "pod112", true));
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Chosen {
    panel: String,
    row: String,
    current: bool,
    fields: Vec<(String, String)>,
}

impl Chosen {
    /// The row `row` of the panel `panel`, by the side's ids, chosen from the panel as it is now, as `current` says.
    pub fn new(panel: impl Into<String>, row: impl Into<String>, current: bool) -> Self {
        Self {
            panel: panel.into(),
            row: row.into(),
            current,
            fields: Vec::new(),
        }
    }

    /// The choice, of a row whose key is `fields`, as the side published it.
    pub fn with_fields(mut self, fields: Vec<(String, String)>) -> Self {
        self.fields = fields;
        self
    }

    /// The key of the row chosen, verbatim, in the order the side gave its fields ([`crate::Row::with_field`]), as it
    /// was when the participant saw the row: the server hands on a choice only of a row whose key has not changed since
    /// they saw it, and refuses the rest as stale, so that the side acts on exactly what was chosen. None for a row
    /// without a key. A client never sends a key, so none can be forged. See `design/panel-actions.md`.
    pub fn fields(&self) -> &[(String, String)] {
        &self.fields
    }

    /// The value of the chosen row's field `name`, if its key has one.
    ///
    /// ```
    /// use tomori_environment::Chosen;
    ///
    /// let chosen = Chosen::new("queues", "pod-a", true).with_fields(vec![("node".into(), "n-3".into())]);
    /// assert_eq!((chosen.field("node"), chosen.field("region")), (Some("n-3"), None));
    /// assert!(Chosen::new("queues", "pod-a", true).fields().is_empty());
    /// ```
    pub fn field(&self, name: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|(field, _)| field == name)
            .map(|(_, value)| value.as_str())
    }

    /// The side's id for the panel, as it published it.
    pub fn panel(&self) -> &str {
        &self.panel
    }

    /// The row's id, as the side published it.
    pub fn row(&self) -> &str {
        &self.row
    }

    /// Whether the participant chose it from the panel as it is now: false where the panel has changed since the version
    /// they saw, as when the side reordered its rows, though the row they chose, named by its id, is still on it.
    pub fn current(&self) -> bool {
        self.current
    }
}

impl Intent {
    /// The intent `payload` of the participant numbered `participant`.
    pub fn new(participant: u32, payload: impl Into<String>) -> Self {
        Self {
            participant,
            payload: payload.into(),
            chosen: None,
            acted: None,
            moved: None,
            occupancy: None,
            context: None,
        }
    }

    /// The participant numbered `participant`'s choice, `chosen`, of a row of a list panel the side published, whose
    /// payload is empty. The server makes it, as a participant chooses a row, never from a client's intent; a side's own
    /// tests make it to try their handling of a choice.
    ///
    /// ```
    /// use tomori_environment::{Chosen, Intent};
    ///
    /// let intent = Intent::choice(3, Chosen::new("queue", "pod112", true));
    /// assert_eq!(intent.chosen().map(Chosen::row), Some("pod112"));
    /// assert_eq!((intent.participant(), intent.payload()), (3, ""));
    /// assert_eq!(Intent::new(3, "{}").chosen(), None);
    /// ```
    pub fn choice(participant: u32, chosen: Chosen) -> Self {
        Self {
            participant,
            payload: String::new(),
            chosen: Some(chosen),
            acted: None,
            moved: None,
            occupancy: None,
            context: None,
        }
    }

    /// The action `acted` the space's agent took, on the word of the participant numbered `participant`, or `0`, the
    /// number the server gives nobody, where nobody's word stands behind it; its payload is empty. The server makes it,
    /// as the agent takes an action the side declared on something it describes, never from a client's intent; a side's
    /// own tests make it to try their handling of an action.
    ///
    /// ```
    /// use tomori_environment::{Acted, Intent};
    ///
    /// let intent = Intent::action(3, Acted::new(1, "hen/pip", "peck", "{}"));
    /// assert_eq!(intent.acted().map(Acted::action), Some("peck"));
    /// assert_eq!((intent.participant(), intent.payload(), intent.chosen()), (3, "", None));
    /// assert_eq!(Intent::new(3, "{}").acted(), None);
    /// ```
    pub fn action(participant: u32, acted: Acted) -> Self {
        Self {
            participant,
            payload: String::new(),
            chosen: None,
            acted: Some(acted),
            moved: None,
            occupancy: None,
            context: None,
        }
    }

    /// A change, `moved`, of where a panel the side published is, made by the participant numbered `participant`, or
    /// `0`, the number the server gives nobody, for one the layout, the agent or the side's own publishing made, whose
    /// payload is empty. The server makes it, never from a client's intent; a side's own tests make it to try their
    /// handling of a move.
    ///
    /// ```
    /// use tomori_environment::{Intent, Moved, PanelPlace};
    ///
    /// let intent = Intent::move_of(3, Moved::new("system", PanelPlace::Wall { wall: 2, slot: 0 }));
    /// assert_eq!(intent.moved().map(Moved::place), Some(PanelPlace::Wall { wall: 2, slot: 0 }));
    /// assert_eq!((intent.participant(), intent.payload(), intent.chosen()), (3, "", None));
    /// assert_eq!(Intent::new(3, "{}").moved(), None);
    /// ```
    pub fn move_of(participant: u32, moved: Moved) -> Self {
        Self {
            participant,
            payload: String::new(),
            chosen: None,
            acted: None,
            moved: Some(moved),
            occupancy: None,
            context: None,
        }
    }

    /// The occupancy of one of the side's zones, `occupancy`, which nobody asked, so its participant is `0`, the number
    /// the server gives nobody, and its payload is empty. The server makes it, as a zone is declared and as how many
    /// are within it changes, never from a client's intent; a side's own tests make it to try their handling of a zone.
    ///
    /// ```
    /// use tomori_environment::{Intent, Occupancy};
    ///
    /// let intent = Intent::zone(Occupancy::new("coop", 1));
    /// assert_eq!(intent.occupancy().map(Occupancy::count), Some(1));
    /// assert_eq!((intent.participant(), intent.payload(), intent.acted()), (0, "", None));
    /// assert_eq!(Intent::new(3, r#"{"zone":"coop"}"#).occupancy(), None);
    /// ```
    pub fn zone(occupancy: Occupancy) -> Self {
        Self {
            participant: 0,
            payload: String::new(),
            chosen: None,
            acted: None,
            moved: None,
            occupancy: Some(occupancy),
            context: None,
        }
    }

    /// How one of the side's panels is bound to its context now, `context`, changed by the participant numbered
    /// `participant`, or `0`, the number the server gives nobody, for one the agent or the layout changed or the side
    /// started again; its payload is empty. The server makes it, never from a client's intent; a side's own tests make
    /// it to try their handling of a context. See `design/panel-context.md`.
    ///
    /// ```
    /// use tomori_environment::{Binding, Context, Intent, Time};
    ///
    /// let context = Context::new("latency", vec![("region".into(), Binding::Fixed("EU-WEST1".into()))], Time::Follows);
    /// let intent = Intent::context_of(0, context.clone());
    /// assert_eq!((intent.context(), intent.payload(), intent.moved()), (Some(&context), "", None));
    /// assert_eq!(Intent::new(3, "{}").context(), None);
    /// ```
    pub fn context_of(participant: u32, context: Context) -> Self {
        Self {
            participant,
            payload: String::new(),
            chosen: None,
            acted: None,
            moved: None,
            occupancy: None,
            context: Some(context),
        }
    }

    /// How one of the side's panels is bound to its context now, if this tells it rather than being the client side's
    /// own intent. Only the server tells one, as the setting of the panel or its wall changes, and for each panel bound
    /// to anything but following as the side starts: a client's intent is never one, whatever its payload says.
    pub fn context(&self) -> Option<&Context> {
        self.context.as_ref()
    }

    /// Who asked: the number the server gave the participant, which everyone in the space knows them by, as the
    /// `source` of their audio and presence. The same participant has the same number for as long as the server keeps
    /// them.
    pub fn participant(&self) -> u32 {
        self.participant
    }

    /// The environment's own intent, as JSON text: empty for a choice, which a side that knows nothing of choices reads
    /// as it reads any intent it cannot, and ignores.
    pub fn payload(&self) -> &str {
        &self.payload
    }

    /// The row of one of the side's list panels the participant chose, if this is a choice rather than the client
    /// side's own intent. Only the server makes a choice, as a participant chooses a row of a panel this side published
    /// in this space: a client's intent is never one, whatever its payload says, so a side may act on a choice as a
    /// participant's own, as on anything they asked, validating the row against its own state.
    pub fn chosen(&self) -> Option<&Chosen> {
        self.chosen.as_ref()
    }

    /// The action the space's agent took on something this side describes, if this is one rather than the client
    /// side's own intent. Only the server makes one, once it has checked it against what the side declared and the
    /// object offers now, and checked who asked as the action needs: a client's intent is never one, whatever its
    /// payload says. The side answers it with [`Space::answer`], by its [`Acted::ticket`], still validating it against
    /// its own state. See `design/environment-actions.md`.
    pub fn acted(&self) -> Option<&Acted> {
        self.acted.as_ref()
    }

    /// A change of where one of the side's panels is, if this is one rather than the client side's own intent: onto
    /// another wall, off its wall to float, or dismissed, by the participant [`Intent::participant`] names, or by
    /// nobody, `0`, for the layout's, the agent's or the side's own. Only the server makes one: a client's intent is
    /// never one, whatever its payload says. A side may then keep updating the panel where it is, as for something else,
    /// retract it, or publish another in its place. See `design/panel-actions.md`.
    pub fn moved(&self) -> Option<&Moved> {
        self.moved.as_ref()
    }

    /// How many are within one of the side's zones, if this is the server telling it so rather than the client side's
    /// own intent. Only the server makes one, as it looks at where participants are in a zone this side declared: a
    /// client's intent is never one, whatever its payload says. See [`Space::declare_zones`].
    pub fn occupancy(&self) -> Option<&Occupancy> {
        self.occupancy.as_ref()
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
/// assert_eq!(object.position(), None);
/// let object = object.at([2.0, 1.2, -4.0]);
/// assert_eq!(object.position(), Some([2.0, 1.2, -4.0]));
/// ```
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Object {
    id: String,
    description: String,
    reference: Option<String>,
    position: Option<[f32; 3]>,
    extent: Option<f32>,
    offers: Vec<String>,
}

/// The least and the most an object reaches from its middle, in metres, as [`Object::with_extent`] says it, and what it
/// is taken to without one.
pub const EXTENT_LIMITS: (f32, f32) = (0.05, 3.0);
pub const DEFAULT_EXTENT: f32 = 0.15;

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

/// How many updates, states it shares and events it announces, a side may make each second in its space, after a burst
/// of [`UPDATE_BURST`]: the server refuses those beyond, saying so, as it does panels published too fast. Enough for a
/// side that says where what moves is 20 times a second, with room for what else changes.
pub const UPDATES_PER_SECOND: u32 = 30;
pub const UPDATE_BURST: u32 = 60;

/// How often, at most, the server tells a side where a participant is, [`Space::presence`]: 20 times a second, however
/// often their client reports it.
pub const PRESENCE_INTERVAL: Duration = Duration::from_millis(50);

/// The most objects a side describes with [`Space::describe`] that the server keeps: those after are left out. Any
/// object kept may be pointed at, and, by its reference, inspected and acted on; the server lists fewer to the space's
/// agent at once, those most relevant to whoever spoke, whatever order the side describes them in.
pub const OBJECT_LIMIT: usize = 1024;

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
            position: None,
            extent: None,
            offers: Vec::new(),
        }
    }

    /// This object, standing at `position`, in the environment's own coordinates, as the side knows it now, such as a
    /// rack's middle or a hen where she pecks: so that the agent stands off the line of sight to it when someone points
    /// at it, goes to it when asked, and stands a worker by it. A side whose object moves describes it again where it
    /// is now; what it no longer describes has no position. The server carries it into the space's coordinates and
    /// leaves out one that is not finite or stands more than 30 m from home on any axis, describing the object without
    /// it. See `design/environments.md`.
    pub fn at(mut self, position: [f32; 3]) -> Self {
        self.position = Some(position);
        self
    }

    /// This object, which the space's agent can inspect by `reference`, unique among the objects the side describes
    /// and the same for as long as the object is: the side's [`Inspector`] is asked about it by this reference. One
    /// that [`valid_reference`] refuses is left out by the server, which describes the object without it.
    pub fn with_reference(mut self, reference: impl Into<String>) -> Self {
        self.reference = Some(reference.into());
        self
    }

    /// This object, offering the actions the side declared by the names `actions`, after any it offers already, which
    /// the space's agent may take on it, by its reference: so only an object with a reference offers any. The server
    /// keeps at most [`OFFER_LIMIT`], leaving out a name [`valid_action_name`] refuses or given twice, and offers only
    /// those the side has declared, with [`Space::declare_actions`]. A side whose object stops taking an action, as a
    /// terminal frozen during an incident, describes it again without it. See `design/environment-actions.md`.
    ///
    /// ```
    /// use tomori_environment::Object;
    ///
    /// let terminal = Object::new("terminal-2", "deck terminal 2").with_reference("terminal/2").offering(["set_region"]);
    /// assert_eq!(terminal.offers(), ["set_region"]);
    /// ```
    pub fn offering(mut self, actions: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.offers.extend(actions.into_iter().map(Into::into));
        self
    }

    /// The names of the actions it offers, as the side gave them.
    pub fn offers(&self) -> &[String] {
        &self.offers
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

    /// Where it stands, in the environment's own coordinates, if the side says.
    pub fn position(&self) -> Option<[f32; 3]> {
        self.position
    }

    /// This object, `radius` metres across from its middle, as a diagram's leader line to it rings it: from
    /// [`EXTENT_LIMITS`]'s least to its most, the server holding one beyond them to them and leaving out one that is not
    /// finite. Without it, an object is taken as [`DEFAULT_EXTENT`]. See `design/diagrams.md`.
    ///
    /// ```
    /// use tomori_environment::Object;
    ///
    /// let fire = Object::new("bonfire", "the campsite's bonfire").with_extent(0.6);
    /// assert_eq!(fire.extent(), Some(0.6));
    /// ```
    pub fn with_extent(mut self, radius: f32) -> Self {
        self.extent = Some(radius);
        self
    }

    /// How far it reaches from its middle, in metres, if the side says.
    pub fn extent(&self) -> Option<f32> {
        self.extent
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

/// Where a participant is, as their presence last said and the server last told the side: their head, and each
/// controller's grip pose, its −Z forward through the fist, if their presence says it, as the desktop's does not; all in
/// the environment's own coordinates, as the server carries them from the space's; and when the server told the side.
/// See [`Space::presence`].
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct Presence {
    pub participant: u32,
    pub head: tomori_protocol::Pose,
    pub left: Option<tomori_protocol::Pose>,
    pub right: Option<tomori_protocol::Pose>,
    pub at: Instant,
    /// How engaged they are, as their client last reported, if it says.
    pub engagement: Option<Engagement>,
}

impl Presence {
    /// Participant `participant`'s presence, their head at `head` and their controllers at `left` and `right`, if their
    /// presence says them, at `at`.
    pub fn new(
        participant: u32,
        head: tomori_protocol::Pose,
        left: Option<tomori_protocol::Pose>,
        right: Option<tomori_protocol::Pose>,
        at: Instant,
    ) -> Self {
        Self {
            participant,
            head,
            left,
            right,
            at,
            engagement: None,
        }
    }

    /// This presence, with the engagement their client last reported, if it says.
    ///
    /// ```
    /// use std::time::Instant;
    /// use tomori_environment::{Engagement, Presence};
    /// use tomori_protocol::Pose;
    ///
    /// let ada = Presence::new(3, Pose::HOME, None, None, Instant::now()).with_engagement(Some(Engagement::Away));
    /// assert_eq!(ada.engagement, Some(Engagement::Away));
    /// ```
    pub fn with_engagement(mut self, engagement: Option<Engagement>) -> Self {
        self.engagement = engagement;
        self
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
    /// Place the cluster anchor `anchor`, or move, resize or relabel the one the side placed with its name in place. An
    /// outlet that takes no anchors, as one made before anchors were, keeps nothing.
    fn place_anchor(&self, anchor: Anchor) -> Result<()> {
        let _ = anchor;
        Ok(())
    }
    /// Remove the cluster anchor the side placed as `name`, if it is there.
    fn remove_anchor(&self, name: &str) -> Result<()> {
        let _ = name;
        Ok(())
    }
    /// Declare the actions the space's agent may take on what the side describes, replacing those declared before. An
    /// outlet that takes no actions, as one made before actions were, keeps nothing.
    fn declare_actions(&self, actions: Vec<Action>) -> Result<()> {
        let _ = actions;
        Ok(())
    }
    /// Declare the zones whose occupancy the side is told, replacing those declared before. An outlet that takes no
    /// zones, as one made before zones were, keeps nothing.
    fn declare_zones(&self, zones: Vec<Zone>) -> Result<()> {
        let _ = zones;
        Ok(())
    }
    /// Declare the parameters the side's panels depend on, with their values now, replacing those declared before. An
    /// outlet that takes no context, as one made before context was, keeps nothing.
    fn set_context(&self, parameters: Vec<ContextParameter>) -> Result<()> {
        let _ = parameters;
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
    answers: Mutex<Answers>,
    answered: Condvar,
    /// The participants in the space, by their numbers, as the server last said, or nothing until it has.
    present: Mutex<Option<Vec<u32>>>,
    /// Where each participant is, by their number, as the server last told the side.
    presence: Mutex<HashMap<u32, Presence>>,
}

/// The actions handed on to a side that wait for its answer, and the answers it gave that the server has not yet taken.
#[derive(Default)]
struct Answers {
    /// The ticket the next action is given, counting from 1.
    next: u64,
    waiting: HashSet<u64>,
    given: HashMap<u64, Outcome>,
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

    /// Hand on the action `action` taken on the object of `reference` with `parameters`, as the server checked them, on
    /// the word of the participant numbered `participant`, unless [`WAITING_INTENT_LIMIT`] intents are already waiting,
    /// when it is dropped. Returns the ticket the side answers it by, which [`Inbox::outcome`] waits for, or `None` if it
    /// was dropped.
    pub fn act(
        &self,
        participant: u32,
        reference: &str,
        action: &str,
        parameters: &str,
    ) -> Option<u64> {
        let mut answers = self
            .0
            .answers
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let ticket = answers.next + 1;
        let acted = Acted::new(ticket, reference, action, parameters);
        if !self.push(Intent::action(participant, acted)) {
            return None;
        }
        answers.next = ticket;
        answers.waiting.insert(ticket);
        Some(ticket)
    }

    /// The side's answer to the action handed on as `ticket`, waiting for it for at most `timeout`; `None` if it did not
    /// answer in time, after which an answer it gives is not taken.
    pub fn outcome(&self, ticket: u64, timeout: Duration) -> Option<Outcome> {
        let answers = self
            .0
            .answers
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let (mut answers, _) = self
            .0
            .answered
            .wait_timeout_while(answers, timeout, |answers| {
                !answers.given.contains_key(&ticket)
            })
            .unwrap_or_else(PoisonError::into_inner);
        answers.waiting.remove(&ticket);
        answers.given.remove(&ticket)
    }

    /// Say who takes part in the space now, `participants`, by the numbers the server gives them, which
    /// [`Space::present`] reads; visitors are not among them. The server says as it sweeps, whether or not anyone came or
    /// went.
    ///
    /// ```
    /// use tomori_environment::Space;
    ///
    /// let space = Space::new("space-1-1", "Yard", "/");
    /// assert_eq!(space.present(), None);
    /// space.inbox().set_present(vec![7, 3, 7]);
    /// assert_eq!(space.present(), Some(vec![3, 7]));
    /// ```
    pub fn set_present(&self, mut participants: Vec<u32>) {
        participants.sort_unstable();
        participants.dedup();
        // Nobody who has gone is anywhere.
        self.0
            .presence
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|participant, _| participants.binary_search(participant).is_ok());
        *self
            .0
            .present
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(participants);
    }

    /// Tell the side how many are within one of its zones, `occupancy`, unless [`WAITING_INTENT_LIMIT`] intents are
    /// already waiting, when it is not told. Says whether it was, so that the server tells it again later.
    pub fn occupy(&self, occupancy: Occupancy) -> bool {
        self.push(Intent::zone(occupancy))
    }

    /// Say where a participant is now, `presence`, as their client reported it, which [`Space::presence`] reads: only
    /// once [`PRESENCE_INTERVAL`] has passed since the last it took for them, by its [`Presence::at`], so that a side is
    /// told at most 20 times a second however often a headset reports. Says whether it took it. The server says it of
    /// participants only, never visitors.
    ///
    /// ```
    /// use std::time::{Duration, Instant};
    /// use tomori_environment::{Presence, Space};
    /// use tomori_protocol::Pose;
    ///
    /// let space = Space::new("space-1-1", "Yard", "/");
    /// let now = Instant::now();
    /// let head = Pose::at([0.0, 1.6, 0.0]);
    /// assert!(space.inbox().set_presence(Presence::new(3, head, None, None, now)));
    /// assert!(!space.inbox().set_presence(Presence::new(3, head, None, None, now + Duration::from_millis(10))));
    /// assert_eq!(space.presence().len(), 1);
    /// ```
    pub fn set_presence(&self, presence: Presence) -> bool {
        let mut known = self
            .0
            .presence
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let soon = known
            .get(&presence.participant)
            .is_some_and(|last| presence.at.saturating_duration_since(last.at) < PRESENCE_INTERVAL);
        if soon {
            return false;
        }
        known.insert(presence.participant, presence);
        true
    }

    /// Say that participant `participant` is now `engagement`, as their client reported it, in where [`Space::presence`]
    /// says they are, at once, whatever [`PRESENCE_INTERVAL`] holds back, since a headset lifted off says it is away and
    /// then reports nothing more. Changes nothing for a participant the side has not been told is present.
    ///
    /// ```
    /// use std::time::Instant;
    /// use tomori_environment::{Engagement, Presence, Space};
    /// use tomori_protocol::Pose;
    ///
    /// let space = Space::new("space-1-1", "Yard", "/");
    /// space.inbox().set_presence(Presence::new(3, Pose::HOME, None, None, Instant::now()));
    /// space.inbox().set_engagement(3, Engagement::Away);
    /// space.inbox().set_engagement(4, Engagement::Away);
    /// assert_eq!(space.presence()[0].engagement, Some(Engagement::Away));
    /// assert_eq!(space.presence().len(), 1);
    /// ```
    pub fn set_engagement(&self, participant: u32, engagement: Engagement) {
        if let Some(known) = self
            .0
            .presence
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_mut(&participant)
        {
            known.engagement = Some(engagement);
        }
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
    /// larger than [`STATE_LIMIT`], saying why; and, as it does states shared, more than [`UPDATES_PER_SECOND`], 30, a
    /// second, after a burst of [`UPDATE_BURST`], 60, saying so, so that a side gathers what changed into one event
    /// rather than announcing each change on its own.
    pub fn announce(&self, event: impl Into<String>, state: impl Into<String>) -> Result<()> {
        self.outlet.announce(event.into(), state.into())
    }

    /// Describe the objects a participant can point at in the space, replacing those described before, so that the
    /// server can tell the space's agent what a participant points at as they speak. The server keeps at most
    /// [`OBJECT_LIMIT`], 1,024, any of which may be pointed at, inspected and acted on, whatever their order, each
    /// identity at most [`IDENTITY_LIMIT`], 128 bytes, and each description cleaned and cut to [`DESCRIPTION_LIMIT`],
    /// 200 characters, with its reference, if it has one that [`valid_reference`] takes, of at most
    /// [`REFERENCE_BYTES`], which the agent may inspect it by (see [`Space::inspect_with`]).
    pub fn describe(&self, objects: Vec<Object>) -> Result<()> {
        self.outlet.describe(objects)
    }

    /// Publish `panel` onto the environment's own wall it names, for everyone in the space, as the agent's panels are
    /// shown: or, where the side published a panel with its id before, update that panel in place, replacing what it
    /// shows and keeping where it is, wherever a participant or the agent moved it. Participants and the agent move,
    /// point at and dismiss it as any panel, every change of the wall it is on and its dismissal, whatever made them,
    /// reaching the side as an [`Intent::moved`]; one dismissed stays away, its updates ignored, until the side
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

    /// Declare the parameters the side's panels depend on, as its own selection gives them now, replacing those declared
    /// before, such as the region its console has selected: a panel that names one in [`Panel::depends_on`] and follows
    /// it shows its value, and a wall or panel that fixes it to another value is shown so, the side being told
    /// ([`Intent::context`]) and showing in that panel what its context says. Call it again as the selection changes;
    /// every participant sees the values on the walls' and panels' chips. A setting of a parameter this leaves out is kept, shown nowhere, and its panels told no binding for it until the side declares it again; a value fixed that is no longer among the choices is kept and still shown and told as fixed. Declarations belong to the binding, as
    /// everything a side keeps does, so a side declares them again as it starts. See `design/panel-context.md`.
    ///
    /// The server refuses, saying why: more than [`CONTEXT_PARAMETERS`] parameters, an id [`valid_context_id`] refuses
    /// or another parameter has, and a parameter of more than [`CONTEXT_CHOICES`] choices. It cleans each label, value and
    /// choice onto one line, cutting a label after [`CONTEXT_LABEL_CHARS`] and a value after [`CONTEXT_VALUE_CHARS`]
    /// characters, a label empty once cleaned taking its id, and holds declarations to the rate its publishing is held
    /// to.
    ///
    /// ```
    /// use tomori_environment::{ContextParameter, Space};
    ///
    /// let space = Space::new("space-1-1", "Fleet", "/");
    /// space.set_context(vec![ContextParameter::new("region", "Region", "US-EAST1")]).unwrap();
    /// ```
    pub fn set_context(&self, parameters: Vec<ContextParameter>) -> Result<()> {
        self.outlet.set_context(parameters)
    }

    /// Place `anchor`, a named place in the environment that workers, panels and the agent gather at, in the
    /// environment's own coordinates, as its state changes, such as a rack that starts alerting: or, given the name of
    /// one the side placed before, move, resize or relabel that one in place, what stands there gliding with it. Choose
    /// names that stay the same for as long as the place is, so that what the space keeps there by name finds it again.
    /// What the side places belongs to its binding, as everything it keeps does, so a side places its anchors again as
    /// it starts. See `design/environments.md`.
    ///
    /// The server refuses, saying why: a name, extent, kind or object [`Anchor::checked`] refuses; the name of one the
    /// environment declares, `home` among them; a pose not finite, without an orientation, or more than 30 m from home
    /// on any axis; and a new one beyond [`ANCHOR_LIMIT`], 64, in the space, those it declares counted first. It cleans
    /// the label onto one line and cuts it after [`ANCHOR_LABEL_LIMIT`] characters. It makes each anchor's changes at
    /// most about once every [`ANCHOR_INTERVAL`], and [`ANCHOR_CHANGES_PER_SECOND`] in the space after a burst of
    /// [`ANCHOR_BURST`]: a change that comes too soon waits, the latest replacing the one before, and a removal cancels
    /// a placement still waiting. A side that debounces its own state, so that a flapping alert does not move what
    /// stands there to and fro, is the better fix.
    ///
    /// ```
    /// use tomori_environment::{Anchor, Space};
    /// use tomori_protocol::Pose;
    ///
    /// let space = Space::new("space-1-1", "Fleet", "/");
    /// let rack = Anchor::new("rack/a12", Pose::at([4.0, 1.4, -6.0]))
    ///     .with_extent(2.0, 2.5, 1.0)
    ///     .with_kind("rack")
    ///     .with_label("Rack A12, alerting: CPU throttled");
    /// space.place_anchor(rack).unwrap();
    /// space.remove_anchor("rack/a12").unwrap();
    /// ```
    pub fn place_anchor(&self, anchor: Anchor) -> Result<()> {
        self.outlet.place_anchor(anchor)
    }

    /// Remove the anchor the side placed as `name`: its workers go home, its panels stay floating where they stand, and
    /// a notice on the status wall says so. One not there changes nothing.
    pub fn remove_anchor(&self, name: &str) -> Result<()> {
        self.outlet.remove_anchor(name)
    }

    /// Declare the actions the space's agent may take on what this side describes, replacing those declared before:
    /// each object offers those it applies to by name, with [`Object::offering`], and each the agent takes reaches the
    /// side as an [`Intent`] whose [`Intent::acted`] names it, which the side answers with [`Space::answer`]. The server
    /// refuses, saying why, a declaration [`check_actions`] refuses, and keeps none of it; it cleans each description
    /// and label onto one line and cuts it after [`ACTION_DESCRIPTION_LIMIT`], [`PARAMETER_DESCRIPTION_LIMIT`] or
    /// [`CHOICE_LABEL_LIMIT`] characters. What it keeps belongs to the side's binding. See
    /// `design/environment-actions.md`.
    ///
    /// ```
    /// use tomori_environment::{Action, Parameter, Space};
    ///
    /// let space = Space::new("space-1-1", "Yard", "/");
    /// let pecks = Parameter::number("pecks", "how many times", 1.0, 8.0).stepped(1.0);
    /// space.declare_actions(vec![Action::new("peck", "Have this hen peck at the ground").with(pecks)]).unwrap();
    /// ```
    pub fn declare_actions(&self, actions: Vec<Action>) -> Result<()> {
        self.outlet.declare_actions(actions)
    }

    /// Declare the zones whose occupancy this side is told, replacing those declared before, such as when a console's
    /// terminal changes: each a sphere or a vertical cylinder about a point in the environment's own coordinates, or
    /// about an anchor or an object it gives, which the server follows wherever it stands. As the hub sweeps, about once
    /// a second, the server counts the participants whose heads are within each, entering at its radius and leaving
    /// beyond its margin, as their clients last reported them, none whose report is [`PRESENCE_STALE`] or older, none
    /// whose client says they are away, and no visitor; and tells the side, as an [`Intent`] whose [`Intent::occupancy`] names the zone and how many are within
    /// it, each zone's occupancy at the next sweep after it is declared, whatever it is, and then each change of how
    /// many: a change between nobody and somebody only once it has lasted the zone's dwell, any other at once. Who they
    /// are it is not told; [`Space::presence`] says where each is, by number. An occupancy waits in the inbox in order with
    /// intents, and one the inbox has no room for is told at a later sweep. A zone declared again under its name keeps
    /// who was within it; a declaration sooner than [`ZONE_INTERVAL`] after the last taken waits, the latest replacing the
    /// one before. The server refuses, saying why, a
    /// declaration [`check_zones`] refuses, or one around a point beyond 30 m of home once carried into the space, and
    /// keeps none of it. What it keeps belongs to the side's binding. See `design/environments.md`.
    ///
    /// ```
    /// use tomori_environment::{Around, Space, Zone};
    ///
    /// let space = Space::new("space-1-1", "Deck", "/");
    /// space.declare_zones(vec![Zone::cylinder("console/2", Around::anchor("console/2"), 1.5, 0.5, 2.5)]).unwrap();
    /// ```
    pub fn declare_zones(&self, zones: Vec<Zone>) -> Result<()> {
        self.outlet.declare_zones(zones)
    }

    /// Answer the action handed to this side as `ticket`, [`Acted::ticket`], with `outcome`: done, with a summary of
    /// what changed, or refused, with the reason. The server waits for an answer for at most a few seconds, telling the
    /// agent then that the environment did not answer; an answer to an action no longer waited for, as one answered
    /// already or too late, is not taken. Returns whether it was.
    ///
    /// ```
    /// use std::time::Duration;
    /// use tomori_environment::{Outcome, Space};
    ///
    /// let space = Space::new("space-1-1", "Yard", "/");
    /// let ticket = space.inbox().act(3, "hen/pip", "peck", r#"{"pecks":2}"#).unwrap();
    /// let intent = space.next_intent(Duration::ZERO).unwrap();
    /// assert_eq!(intent.acted().unwrap().ticket(), ticket);
    /// assert!(space.answer(ticket, Outcome::done("Pip pecks at the ground.")));
    /// assert_eq!(space.inbox().outcome(ticket, Duration::ZERO), Some(Outcome::done("Pip pecks at the ground.")));
    /// assert!(!space.answer(ticket, Outcome::done("again")));
    /// ```
    pub fn answer(&self, ticket: u64, outcome: Outcome) -> bool {
        let mut answers = self
            .waiting
            .answers
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if !answers.waiting.remove(&ticket) {
            return false;
        }
        answers.given.insert(ticket, outcome);
        self.waiting.answered.notify_all();
        true
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

    /// Who takes part in the space now, by the numbers the server gives participants, which [`Intent::participant`]
    /// names them by, in order: as the server last said, as it sweeps, about once a second, so that a side can let go of
    /// what someone who has left held, or tidy up once nobody is there. Visitors, who send no intents, are not among
    /// them, nor is anyone whose connection has ended. `None` until the server has said, as for a space made without
    /// one or by a server from before, when a side should assume nothing.
    pub fn present(&self) -> Option<Vec<u32>> {
        self.waiting
            .present
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Where each participant in the space is, by their numbers, in order, as the server last told the side, at most 20
    /// times a second ([`PRESENCE_INTERVAL`]), in the environment's own coordinates: so that a side can check that what a
    /// participant asks for is within their reach, and know where their hand holds what they hold. Participants only,
    /// never visitors, and only while they are present, as [`Space::present`] says; one whose presence the server has
    /// not been told, as a client that reports none, is not among them. Each says when it was told, so that a side can
    /// tell someone whose presence has gone quiet.
    pub fn presence(&self) -> Vec<Presence> {
        let mut presence: Vec<Presence> = self
            .waiting
            .presence
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
            .copied()
            .collect();
        presence.sort_by_key(|presence| presence.participant);
        presence
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
    fn an_object_stands_and_reaches_where_its_side_says() {
        let object = Object::new("bonfire", "the campsite's bonfire");
        assert_eq!((object.position(), object.extent()), (None, None));
        let object = object.at([2.0, 0.0, -4.0]).with_extent(0.6);
        assert_eq!(object.position(), Some([2.0, 0.0, -4.0]));
        assert_eq!(object.extent(), Some(0.6));
    }

    #[test]
    fn a_space_is_what_it_was_given() {
        let space = Space::new("space-1-2", "Fleet", "/srv/fleet");
        assert_eq!(space.id(), "space-1-2");
        assert_eq!(space.title(), "Fleet");
        assert_eq!(space.directory(), "/srv/fleet");
    }

    #[test]
    fn a_side_is_told_where_each_participant_is_at_most_twenty_times_a_second_while_they_are_present()
     {
        let space = Space::new("space-1-2", "Yard", "/");
        assert!(space.presence().is_empty());
        let now = Instant::now();
        let head = tomori_protocol::Pose::at([1.0, 1.6, 0.0]);
        let hand = tomori_protocol::Pose::at([1.2, 1.1, -0.3]);
        let inbox = space.inbox();
        assert!(inbox.set_presence(Presence::new(7, head, None, Some(hand), now)));
        assert!(inbox.set_presence(Presence::new(2, head, Some(hand), None, now)));
        // Too soon after the last, it is not taken; once the interval has passed, it is.
        let soon = now + PRESENCE_INTERVAL / 2;
        assert!(!inbox.set_presence(Presence::new(7, hand, None, None, soon)));
        let later = now + PRESENCE_INTERVAL;
        assert!(inbox.set_presence(Presence::new(7, hand, None, None, later)));
        let presence = space.presence();
        assert_eq!(
            presence
                .iter()
                .map(|presence| presence.participant)
                .collect::<Vec<_>>(),
            [2, 7]
        );
        assert_eq!(
            (presence[1].head, presence[1].right, presence[1].at),
            (hand, None, later)
        );
        assert_eq!(presence[0].left, Some(hand));
        // Gone from the space, they are nowhere.
        inbox.set_present(vec![7]);
        assert_eq!(space.presence().len(), 1);
        assert_eq!(space.presence()[0].participant, 7);
    }

    #[test]
    fn a_space_made_without_an_outlet_sends_nowhere() {
        let space = Space::new("space-1-2", "Fleet", "/srv/fleet");
        assert_eq!(space.share("{}"), Ok(()));
        assert_eq!(space.announce("{}", "{}"), Ok(()));
        assert_eq!(space.describe(vec![Object::new("a", "b")]), Ok(()));
        assert_eq!(space.publish(Panel::text("a", "b", "c")), Ok(()));
        assert_eq!(space.retract("a"), Ok(()));
        let rack = Anchor::new("rack/a12", tomori_protocol::Pose::HOME);
        assert_eq!(space.place_anchor(rack), Ok(()));
        assert_eq!(space.remove_anchor("rack/a12"), Ok(()));
        assert_eq!(space.declare_actions(vec![Action::new("a", "b")]), Ok(()));
        let well = Zone::sphere("well", Around::point([0.0; 3]), 1.0);
        assert_eq!(space.declare_zones(vec![well]), Ok(()));
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
        // Nor anchors, made before them.
        let rack = Anchor::new("rack/a12", tomori_protocol::Pose::HOME);
        assert_eq!(space.place_anchor(rack), Ok(()));
        assert_eq!(space.remove_anchor("rack/a12"), Ok(()));
        // Nor actions, made before them.
        assert_eq!(space.declare_actions(Vec::new()), Ok(()));
        // Nor zones, made before them.
        assert_eq!(space.declare_zones(Vec::new()), Ok(()));
    }

    #[test]
    fn an_occupancy_is_an_intent_only_the_server_makes_waiting_in_order() {
        let space = Space::new("space-1-7", "Deck", "/");
        let inbox = space.inbox();
        // A client's intent is never an occupancy, whatever it says.
        let forged = Intent::new(4, r#"{"occupancy":{"zone":"console","count":3}}"#);
        assert_eq!(forged.occupancy(), None);
        assert!(inbox.push(forged.clone()));
        assert!(inbox.occupy(Occupancy::new("console", 1)));
        assert_eq!(space.next_intent(Duration::ZERO), Some(forged));
        let told = space.next_intent(Duration::ZERO).unwrap();
        assert_eq!(told.occupancy(), Some(&Occupancy::new("console", 1)));
        assert_eq!(
            (told.participant(), told.chosen(), told.acted()),
            (0, None, None)
        );
        // Not told while the inbox is full, so that the server tells it again.
        for participant in 0..WAITING_INTENT_LIMIT as u32 {
            assert!(inbox.push(Intent::new(participant, "{}")));
        }
        assert!(!inbox.occupy(Occupancy::new("console", 0)));
    }

    #[test]
    fn an_action_is_an_intent_apart_from_any_payload_answered_by_its_ticket() {
        let space = Space::new("space-1-5", "Yard", "/");
        let inbox = space.inbox();
        // A client's intent is never an action, whatever it says.
        let forged = Intent::new(4, r#"{"acted":{"ticket":1,"action":"peck"}}"#);
        assert_eq!(forged.acted(), None);
        assert!(inbox.push(forged.clone()));
        let first = inbox.act(4, "hen/pip", "peck", r#"{"pecks":2}"#).unwrap();
        let second = inbox.act(0, "hen/pip", "peck", r#"{"pecks":3}"#).unwrap();
        assert_eq!((first, second), (1, 2));
        // In order with the client side's intents.
        assert_eq!(space.next_intent(Duration::ZERO), Some(forged));
        let intent = space.next_intent(Duration::ZERO).unwrap();
        assert_eq!(intent.participant(), 4);
        assert_eq!(
            intent.acted(),
            Some(&Acted::new(1, "hen/pip", "peck", r#"{"pecks":2}"#))
        );
        // An answer given before the server waits is kept for it; one not waited for in time is not taken.
        assert!(space.answer(first, Outcome::refused("She is roosting.")));
        assert_eq!(
            inbox.outcome(first, Duration::ZERO),
            Some(Outcome::refused("She is roosting."))
        );
        assert_eq!(inbox.outcome(second, Duration::from_millis(1)), None);
        assert!(!space.answer(second, Outcome::done("late")));
        assert!(!space.answer(99, Outcome::done("never asked")));
        // Answered from the side's thread while the server waits.
        let _second = space.next_intent(Duration::ZERO);
        let ticket = inbox.act(1, "hen/pip", "peck", "{}").unwrap();
        let side = std::thread::spawn(move || {
            let intent = space.next_intent(Duration::from_secs(10)).unwrap();
            space.answer(intent.acted().unwrap().ticket(), Outcome::done("Pecked."))
        });
        assert_eq!(
            inbox.outcome(ticket, Duration::from_secs(10)),
            Some(Outcome::done("Pecked."))
        );
        assert!(side.join().unwrap());
        // Dropped, as any intent, beyond the inbox's limit, and given no ticket.
        let space = Space::new("space-1-6", "Yard", "/");
        for participant in 0..WAITING_INTENT_LIMIT as u32 {
            assert!(space.inbox().push(Intent::new(participant, "{}")));
        }
        assert_eq!(space.inbox().act(1, "hen/pip", "peck", "{}"), None);
        let _ = space.next_intent(Duration::ZERO);
        assert_eq!(space.inbox().act(1, "hen/pip", "peck", "{}"), Some(1));
    }

    #[test]
    fn an_object_offers_the_actions_it_names() {
        let object = Object::new("chicken-1", "Pip").with_reference("hen/pip");
        assert!(object.offers().is_empty());
        let object = object.offering(["peck"]).offering(vec!["look".to_owned()]);
        assert_eq!(object.offers(), ["peck", "look"]);
    }

    #[test]
    fn a_choice_is_an_intent_apart_from_any_payload() {
        let chosen = Chosen::new("queue", "pod/7", false);
        assert_eq!(
            (chosen.panel(), chosen.row(), chosen.current()),
            ("queue", "pod/7", false)
        );
        let intent = Intent::choice(4, chosen.clone());
        assert_eq!(intent.chosen(), Some(&chosen));
        assert_eq!((intent.participant(), intent.payload()), (4, ""));
        // A client's intent is never a choice, whatever it says.
        let forged = Intent::new(4, r#"{"chosen":{"panel":"queue","row":"pod/7"}}"#);
        assert_eq!(forged.chosen(), None);
        // Choices wait in the same inbox, in order with the client side's intents.
        let space = Space::new("space-1-4", "Fleet", "/");
        assert!(space.inbox().push(forged.clone()));
        assert!(space.inbox().push(intent.clone()));
        assert_eq!(space.next_intent(Duration::ZERO), Some(forged));
        assert_eq!(space.next_intent(Duration::ZERO), Some(intent));
    }

    #[test]
    fn a_move_is_an_intent_apart_from_any_payload() {
        let moved = Moved::new("system", PanelPlace::Wall { wall: 3, slot: 1 });
        assert_eq!(
            (moved.panel(), moved.place()),
            ("system", PanelPlace::Wall { wall: 3, slot: 1 })
        );
        let intent = Intent::move_of(4, moved.clone());
        assert_eq!(intent.moved(), Some(&moved));
        assert_eq!((intent.chosen(), intent.acted()), (None, None));
        // A client's intent, a choice and an action are never a move, whatever they say.
        let forged = Intent::new(4, r#"{"moved":{"panel":"system","place":"floating"}}"#);
        assert_eq!(forged.moved(), None);
        assert_eq!(Intent::choice(4, Chosen::new("a", "b", true)).moved(), None);
        // Moves wait in the same inbox, in order with the rest.
        let space = Space::new("space-1-5", "Fleet", "/");
        assert!(space.inbox().push(forged.clone()));
        assert!(space.inbox().push(intent.clone()));
        assert_eq!(space.next_intent(Duration::ZERO), Some(forged));
        assert_eq!(space.next_intent(Duration::ZERO), Some(intent));
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
        assert_eq!(OBJECT_LIMIT, 1024);
        assert_eq!(IDENTITY_LIMIT, 128);
        assert_eq!(DESCRIPTION_LIMIT, 200);
        assert_eq!(REFERENCE_BYTES, 64);
        assert_eq!(INSPECTION_LIMIT, 16 * 1024);
        assert_eq!(ANCHOR_LIMIT, 64);
        assert_eq!(ANCHOR_NAME_BYTES, tomori_protocol::cluster::NAME_BYTES);
        assert_eq!(ANCHOR_LABEL_LIMIT, 80);
        assert_eq!(ANCHOR_INTERVAL, Duration::from_millis(900));
        assert_eq!((ANCHOR_CHANGES_PER_SECOND, ANCHOR_BURST), (4, 8));
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
    fn who_is_present_is_unknown_until_the_server_says_and_then_as_it_last_said() {
        let space = Space::new("space-1-2", "Yard", "/");
        assert_eq!(space.present(), None);
        let inbox = space.inbox();
        inbox.set_present(vec![4, 1, 4]);
        assert_eq!(space.present(), Some(vec![1, 4]));
        // Nobody there is not unknown.
        inbox.set_present(Vec::new());
        assert_eq!(space.present(), Some(Vec::new()));
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
