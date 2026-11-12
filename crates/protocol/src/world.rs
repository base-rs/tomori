// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! References to things already in a space's world: an environment's object, a cluster anchor, a wall, a panel, a
//! worker, a participant or the agent, each named as the agent's tools already name it. A diagram binds a short name to
//! one with a line such as `fire := object farmyard:campsite/bonfire`, then draws edges to it; later, callouts share
//! them. The server resolves which thing a reference means, within its space alone, and sends each as a [`Target`]
//! with where it stands, which a client draws a leader line to and marks. See `design/diagrams.md`.
use serde::{Deserialize, Serialize};

use crate::chart::Colour;
use crate::text::{cut_bytes, one_line};

/// The longest name of a reference, in bytes, beyond which it is cut.
pub const NAME_BYTES: usize = 128;

/// The longest label drawn at a target, in characters, beyond which it is cut with "…".
pub const LABEL_CHARS: usize = 32;

/// The least and the most radius of a target, in metres, and what one is taken as that its environment gives none.
pub const RADIUS: (f32, f32) = (0.05, 3.0);
pub const DEFAULT_RADIUS: f32 = 0.15;

/// How far from the space's middle a target may stand on any axis, in metres, beyond which a client draws nothing to it.
pub const REACH: f32 = 1_000.0;

/// What kind of thing a reference names. On the wire its name in lower case; one a client does not know, or a
/// reference written without its kind, which the server infers, is [`Kind::Unknown`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// An environment's object, by its id, as `inspect` takes it, such as `farmyard:campsite/bonfire`.
    Object,
    /// A cluster anchor, by its name, as `presence` lists it, such as `pool-lookout`.
    Anchor,
    /// A wall, by its identity or name, as `set_wall` takes it.
    Wall,
    /// A panel, by its id, identity or title, as `panels` and `move_panel` take it.
    Panel,
    /// A worker, by its title or session, as the worker tools take it.
    Worker,
    /// A participant, by their handle, such as `Samuel#11`, or their name where only one has it.
    Person,
    /// The space's agent itself, named by nothing.
    Agent,
    #[default]
    #[serde(other)]
    Unknown,
}

impl Kind {
    /// The kind `word` names, in any case, or `None`.
    ///
    /// ```
    /// use tomori_protocol::world::Kind;
    ///
    /// assert_eq!(Kind::parse("Object"), Some(Kind::Object));
    /// assert_eq!(Kind::parse("participant"), Some(Kind::Person));
    /// assert_eq!(Kind::parse("rack"), None);
    /// ```
    pub fn parse(word: &str) -> Option<Kind> {
        match word.trim().to_ascii_lowercase().as_str() {
            "object" => Some(Kind::Object),
            "anchor" => Some(Kind::Anchor),
            "wall" => Some(Kind::Wall),
            "panel" => Some(Kind::Panel),
            "worker" => Some(Kind::Worker),
            "person" | "participant" => Some(Kind::Person),
            "agent" => Some(Kind::Agent),
            _ => None,
        }
    }

    /// Its name, as a reference writes it: `object`, `anchor`, and so on, or `thing` for one not said.
    pub fn name(self) -> &'static str {
        match self {
            Kind::Object => "object",
            Kind::Anchor => "anchor",
            Kind::Wall => "wall",
            Kind::Panel => "panel",
            Kind::Worker => "worker",
            Kind::Person => "person",
            Kind::Agent => "agent",
            Kind::Unknown => "thing",
        }
    }
}

/// A thing in the world as the agent named it: its kind and its name, kept as written, cleaned onto one line and cut
/// at [`NAME_BYTES`], rather than as what it resolved to, so that what refers to it outlives the thing changing.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Reference {
    #[serde(default)]
    pub kind: Kind,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
}

impl Reference {
    /// The thing of `kind` named `name`, cleaned onto one line and cut at [`NAME_BYTES`].
    ///
    /// ```
    /// use tomori_protocol::world::{Kind, Reference};
    ///
    /// let fire = Reference::new(Kind::Object, " farmyard:campsite/bonfire ");
    /// assert_eq!(fire.name, "farmyard:campsite/bonfire");
    /// assert_eq!(fire.to_string(), "object farmyard:campsite/bonfire");
    /// assert_eq!(Reference::new(Kind::Agent, "").to_string(), "agent");
    /// ```
    pub fn new(kind: Kind, name: &str) -> Self {
        Self {
            kind,
            name: cut_bytes(one_line(name, NAME_BYTES).trim(), NAME_BYTES),
        }
    }
}

impl std::fmt::Display for Reference {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.name.is_empty() {
            formatter.write_str(self.kind.name())
        } else {
            write!(formatter, "{} {}", self.kind.name(), self.name)
        }
    }
}

/// How a target is marked where a leader line ends: by its kind, by default, an object and an anchor with a ring on the
/// ground, a panel and a wall with their frame lit, and a person, a worker and the agent with a halo above them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mark {
    Ring,
    Halo,
    /// Its outline: a panel's or a wall's frame, and a halo about anything without one.
    Outline,
    /// Nothing but the line's end.
    #[serde(rename = "none")]
    Unmarked,
    #[default]
    #[serde(other)]
    ByKind,
}

impl Mark {
    /// The mark `word` names, in any case, or `None`.
    pub fn parse(word: &str) -> Option<Mark> {
        match word.trim().to_ascii_lowercase().as_str() {
            "ring" => Some(Mark::Ring),
            "halo" => Some(Mark::Halo),
            "outline" | "frame" => Some(Mark::Outline),
            "none" => Some(Mark::Unmarked),
            _ => None,
        }
    }

    /// The mark it is on a target of `kind`: itself, or, by its kind, what that kind is marked with.
    ///
    /// ```
    /// use tomori_protocol::world::{Kind, Mark};
    ///
    /// assert_eq!(Mark::ByKind.on(Kind::Object), Mark::Ring);
    /// assert_eq!(Mark::ByKind.on(Kind::Panel), Mark::Outline);
    /// assert_eq!(Mark::ByKind.on(Kind::Person), Mark::Halo);
    /// assert_eq!(Mark::Halo.on(Kind::Object), Mark::Halo);
    /// ```
    pub fn on(self, kind: Kind) -> Mark {
        match (self, kind) {
            (Mark::ByKind, Kind::Object | Kind::Anchor) => Mark::Ring,
            (Mark::ByKind, Kind::Panel | Kind::Wall) => Mark::Outline,
            (Mark::ByKind, _) => Mark::Halo,
            (mark, _) => mark,
        }
    }
}

/// Whether a reference resolved, as the server last resolved it. One a client does not know is unknown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// Found, and where it stands said: drawn.
    Here,
    /// Found, but where it stands not known now, as a stick in someone's hand, a participant away or an anchor not yet
    /// placed: drawn as nothing in this version.
    Absent,
    /// Never found: drawn as nothing.
    #[default]
    #[serde(other)]
    Unknown,
}

/// The frame of a flat target, a panel or a wall: the way it faces, its +Z, and its width and height, in metres.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Frame {
    pub orientation: [f32; 4],
    pub size: [f32; 2],
}

/// A thing in the world that a diagram refers to, as the server resolved it: the short name edges use, what it refers
/// to, how its end is labelled and marked, and, where it is here, where it stands in the space's coordinates, how large
/// it is and, for a panel or a wall, its frame. A client draws a leader to it where it is here, and nothing otherwise.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Target {
    pub id: String,
    pub reference: Reference,
    /// What its end is labelled with where an edge to it has no label of its own: the agent's, or the server's short name
    /// for it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
    #[serde(default, skip_serializing_if = "is_by_kind")]
    pub mark: Mark,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub colour: Option<Colour>,
    #[serde(default)]
    pub state: State,
    /// Its middle, in the space's coordinates, in metres, while it is here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<[f32; 3]>,
    /// Its radius, in metres, within [`RADIUS`].
    #[serde(default = "default_radius")]
    pub radius: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame: Option<Frame>,
}

fn is_by_kind(mark: &Mark) -> bool {
    *mark == Mark::ByKind
}

fn default_radius() -> f32 {
    DEFAULT_RADIUS
}

impl Target {
    /// The target `id`, of `reference`, not yet resolved.
    pub fn new(id: impl Into<String>, reference: Reference) -> Self {
        Self {
            id: id.into(),
            reference,
            label: String::new(),
            mark: Mark::ByKind,
            colour: None,
            state: State::Unknown,
            at: None,
            radius: DEFAULT_RADIUS,
            frame: None,
        }
    }

    /// Where a client draws a leader to it, where it is here and stands within [`REACH`].
    ///
    /// ```
    /// use tomori_protocol::world::{Kind, Reference, State, Target};
    ///
    /// let mut fire = Target::new("fire", Reference::new(Kind::Object, "farmyard:campsite/bonfire"));
    /// assert_eq!(fire.drawn_at(), None);
    /// fire.state = State::Here;
    /// fire.at = Some([1.0, 0.2, -3.0]);
    /// assert_eq!(fire.drawn_at(), Some([1.0, 0.2, -3.0]));
    /// fire.at = Some([f32::NAN, 0.0, 0.0]);
    /// assert_eq!(fire.drawn_at(), None);
    /// ```
    pub fn drawn_at(&self) -> Option<[f32; 3]> {
        self.at.filter(|at| {
            self.state == State::Here
                && at
                    .iter()
                    .all(|axis| axis.is_finite() && axis.abs() <= REACH)
        })
    }

    /// It as a client draws it, whatever a server sent: its label within [`LABEL_CHARS`], its radius within [`RADIUS`],
    /// and a frame of finite size, at most ten metres a side, with a unit orientation, or none.
    pub fn bounded(&self) -> Target {
        let mut target = self.clone();
        target.label = one_line(&target.label, LABEL_CHARS);
        target.radius = if target.radius.is_finite() {
            target.radius.clamp(RADIUS.0, RADIUS.1)
        } else {
            DEFAULT_RADIUS
        };
        target.frame = target.frame.and_then(|frame| {
            let length = frame
                .orientation
                .iter()
                .map(|value| value * value)
                .sum::<f32>()
                .sqrt();
            (length.is_finite() && length > 0.5 && frame.size.iter().all(|side| side.is_finite()))
                .then(|| Frame {
                    orientation: frame.orientation.map(|value| value / length),
                    size: frame.size.map(|side| side.clamp(0.0, 10.0)),
                })
        });
        target
    }
}

#[cfg(test)]
mod tests;
