// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! A volume's diagram: nodes, edges and groups the agent describes, never geometry, with the positions the server laid
//! them out at. A volume is a panel with depth, a [`crate::Panel`] whose `diagram` this is, as a chart panel is one with
//! a chart, and whose body is the diagram's [outline](Diagram::outline), which a client that draws no volumes shows.
//!
//! The agent writes a diagram as a `diagram` block's body, shaped like a Mermaid flowchart ([`parse`]), or as the JSON
//! of the `show_diagram` tool ([`Diagram::from_json`]); both are read through one [`Builder`], which keeps what fits in
//! the bounds below, in the order written, and says what it left out or changed as [`Warning`]s. The server then lays
//! the diagram out, and a client draws the positions it is sent, within [`Diagram::bounded`]. See `design/diagrams.md`.
use serde::{Deserialize, Serialize};

pub use crate::chart::Colour;
use crate::text::{cut_bytes, one_line};
use crate::world::{Kind, Mark, Reference, Target};

mod parse;
pub use parse::parse;

/// The most nodes of a diagram, edges and groups.
pub const NODES: usize = 64;
pub const EDGES: usize = 128;
pub const GROUPS: usize = 12;
/// How deeply groups nest: a group in a group in a group, and no deeper.
pub const NESTING: usize = 3;
/// The longest id of a node or group, in bytes of ASCII letters, digits, `-`, `_` and `.`.
pub const ID_BYTES: usize = 32;
/// The longest label of a node or group, of an edge, and a node's detail, in characters, and a node's reference, in
/// bytes; each is cut with "…".
pub const LABEL_CHARS: usize = 32;
pub const EDGE_LABEL_CHARS: usize = 24;
pub const DETAIL_CHARS: usize = 240;
pub const REF_BYTES: usize = 256;
/// The most edges between one pair of nodes, either way.
pub const PAIR_EDGES: usize = 4;
/// The most edges read to choose the kept from, beyond which they are only counted: enough that what the bounds
/// leave out, between nodes past [`NODES`], to themselves or past [`PAIR_EDGES`], rarely leaves fewer than [`EDGES`].
pub const EDGE_DRAFTS: usize = EDGES * PAIR_EDGES;
/// The most waypoints of an edge, which a client curves it through.
pub const WAYPOINTS: usize = 4;
/// The largest a node is and the heaviest an edge, from 1.
pub const SIZES: u8 = 5;
pub const WEIGHTS: u8 = 3;
/// How wide a node of size 1 is, in millimetres of a volume at its own scale of 1; each size larger adds a quarter.
pub const NODE_MM: f32 = 60.0;
/// The longest side a volume's extent may have, in millimetres, beyond which a client holds it.
pub const EXTENT_MM: u16 = 30_000;
/// The most warnings told, beyond which they are counted.
pub const WARNINGS: usize = 8;
/// The most world references of a diagram, its world edges, and its world edges to any one reference.
pub const REFERENCES: usize = 16;
pub const WORLD_EDGES: usize = 24;
pub const REFERENCE_EDGES: usize = 8;
/// The most world edges read to choose the kept from, beyond which they are only counted.
const WORLD_DRAFTS: usize = WORLD_EDGES * PAIR_EDGES;

/// What a node looks like. On the wire its name in lower case; one a client does not know is a box.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Shape {
    Sphere,
    Cylinder,
    /// A hexagonal prism.
    Prism,
    Diamond,
    /// A small flat panel standing in the volume, facing the viewer, its label its title and its detail below it.
    Card,
    #[default]
    #[serde(other)]
    Box,
}

/// How an edge is drawn. One a client does not know is solid.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Style {
    Dashed,
    Dotted,
    Thick,
    /// Pulses moving from its source to its target.
    Flow,
    #[default]
    #[serde(other)]
    Solid,
}

/// Which ends of an edge have arrowheads. One a client does not know is forward.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Arrow {
    Back,
    Both,
    None,
    #[default]
    #[serde(other)]
    Forward,
}

/// What a group is drawn as. One a client does not know is a cluster.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupKind {
    /// A floor, its sibling layers stacked one above another.
    Layer,
    /// A box drawn around its members.
    Container,
    /// A tray its members stand on.
    #[default]
    #[serde(other)]
    Cluster,
}

/// Which way a layered layout ranks its nodes. One a client does not know is down.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Rank {
    Up,
    Right,
    Left,
    /// From front to back.
    Away,
    #[default]
    #[serde(other)]
    Down,
}

/// How the server lays a diagram out, as the agent hinted. A client draws the positions it is sent, so it reads a
/// layout it does not know as [`Layout::Unknown`] and draws it all the same.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Layout {
    /// In ranks along a direction, for pipelines, dependency graphs and anything with one.
    Layered {
        #[serde(default)]
        direction: Rank,
    },
    /// About a hub node, for a hub and its spokes.
    Radial {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        hub: Option<String>,
    },
    /// By forces, for graphs with no main direction.
    Force,
    /// In rows and columns in the order written.
    Grid,
    #[serde(other)]
    Unknown,
}

impl Default for Layout {
    fn default() -> Self {
        Self::Layered {
            direction: Rank::Down,
        }
    }
}

impl Layout {
    /// The layout `text` names, as a block's `layout:` header or the tool's `layout` says it: `layered` and a direction,
    /// `down`, `up`, `right`, `left` or `away`, or Mermaid's `TB`, `TD`, `BT`, `LR` or `RL`; `radial` and its hub's id;
    /// `force`; or `grid`. `None` for one Tomori does not know.
    ///
    /// ```
    /// use tomori_protocol::diagram::{Layout, Rank};
    ///
    /// assert_eq!(Layout::parse("layered right"), Some(Layout::Layered { direction: Rank::Right }));
    /// assert_eq!(Layout::parse("LR"), Some(Layout::Layered { direction: Rank::Right }));
    /// assert_eq!(Layout::parse("radial hub"), Some(Layout::Radial { hub: Some("hub".into()) }));
    /// assert_eq!(Layout::parse("spiral"), None);
    /// ```
    pub fn parse(text: &str) -> Option<Self> {
        let mut words = text.split_whitespace();
        let first = words.next()?.to_ascii_lowercase();
        let second = words.next();
        if let Some(direction) = rank(&first) {
            return Some(Self::Layered { direction });
        }
        match first.as_str() {
            "layered" | "layers" | "flowchart" | "graph" => Some(Self::Layered {
                direction: match second {
                    None => Rank::Down,
                    Some(word) => rank(&word.to_ascii_lowercase())?,
                },
            }),
            "radial" => Some(Self::Radial {
                hub: second.map(str::to_owned),
            }),
            "force" => Some(Self::Force),
            "grid" => Some(Self::Grid),
            _ => None,
        }
    }
}

/// The direction `word` names, in lower case.
fn rank(word: &str) -> Option<Rank> {
    match word {
        "down" | "tb" | "td" => Some(Rank::Down),
        "up" | "bt" => Some(Rank::Up),
        "right" | "lr" => Some(Rank::Right),
        "left" | "rl" => Some(Rank::Left),
        "away" => Some(Rank::Away),
        _ => None,
    }
}

/// A point in a volume, in millimetres of the volume at its own scale of 1: right, up and towards its front, about its
/// middle.
pub type Point = [i16; 3];

/// A node: what the agent says of it, and where it was laid out.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Node {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub shape: Shape,
    #[serde(default)]
    pub colour: Colour,
    /// How important it is, from 1 to [`SIZES`], drawn as large.
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub size: u8,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub detail: String,
    /// The agent's own pointer back to what it stands for, such as a crate's path, which it is told when the node is
    /// pointed at. Tomori never follows it.
    #[serde(default, rename = "ref", skip_serializing_if = "String::is_empty")]
    pub reference: String,
    /// The group it is in, by the group's index among [`Diagram::groups`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<u8>,
    /// Where its middle stands.
    #[serde(default)]
    pub at: Point,
}

impl Node {
    /// The node `id`, labelled `label`, a grey box of size 1 at the volume's middle.
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            shape: Shape::Box,
            colour: Colour::Grey,
            size: 1,
            detail: String::new(),
            reference: String::new(),
            group: None,
            at: [0; 3],
        }
    }

    /// How wide it is, in millimetres of the volume at its own scale of 1.
    pub fn width(&self) -> f32 {
        NODE_MM * (1.0 + 0.25 * f32::from(self.size.clamp(1, SIZES) - 1))
    }
}

/// An edge between two nodes, by their indices among [`Diagram::nodes`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Edge {
    pub from: u16,
    pub to: u16,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
    #[serde(default)]
    pub direction: Arrow,
    #[serde(default)]
    pub style: Style,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub colour: Option<Colour>,
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub weight: u8,
    /// The points it passes through between its ends, at most [`WAYPOINTS`], which a client curves it through.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub via: Vec<Point>,
}

impl Edge {
    /// A solid edge from the node at `from` to the one at `to`.
    pub fn new(from: u16, to: u16) -> Self {
        Self {
            from,
            to,
            label: String::new(),
            direction: Arrow::Forward,
            style: Style::Solid,
            colour: None,
            weight: 1,
            via: Vec::new(),
        }
    }
}

/// A group of nodes: a cluster, a layer or a container, in at most one parent group, by its index.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Group {
    pub id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
    #[serde(default)]
    pub kind: GroupKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<u8>,
    /// The box about its members, its least and greatest corners.
    #[serde(default)]
    pub low: Point,
    #[serde(default)]
    pub high: Point,
}

impl Group {
    /// The cluster `id`, labelled `label`.
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            kind: GroupKind::Cluster,
            parent: None,
            low: [0; 3],
            high: [0; 3],
        }
    }
}

/// A diagram: its layout hint, nodes, edges and groups, and, once the server has laid it out, where each stands and the
/// volume's extent about its middle, in millimetres at its own scale of 1.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Diagram {
    #[serde(default)]
    pub layout: Layout,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nodes: Vec<Node>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub edges: Vec<Edge>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<Group>,
    /// Its width, height and depth.
    #[serde(default)]
    pub extent: [u16; 3],
    /// The things in the world it refers to, at most [`REFERENCES`], each by the short name its edges use, as the
    /// server resolved them. A client that does not draw world edges is sent none. See `design/diagrams.md`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub world: Vec<Target>,
    /// Its world edges, at most [`WORLD_EDGES`], each from the node at its `from` to the target at its `to`, by their
    /// indices among [`Self::nodes`] and [`Self::world`], its arrow forward where it points at the target: drawn in the
    /// space as a leader line from the node to the thing, without waypoints.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub world_edges: Vec<Edge>,
}

fn one() -> u8 {
    1
}

fn is_one(value: &u8) -> bool {
    *value == 1
}

/// Something read of a diagram that was left out or changed, with the line of a block it came from, from 1, where it
/// came from one, which the agent is told so that it can put it right.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Warning {
    pub line: Option<usize>,
    pub text: String,
}

impl std::fmt::Display for Warning {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.line {
            Some(line) => write!(formatter, "line {line}: {}", self.text),
            None => formatter.write_str(&self.text),
        }
    }
}

/// `warnings` as the agent is told them: the first [`WARNINGS`], one to a line, and how many more.
///
/// ```
/// use tomori_protocol::diagram::{Warning, told};
///
/// let warning = |text: &str| Warning { line: None, text: text.into() };
/// assert_eq!(told(&[warning("a"), warning("b")]), "a\nb");
/// assert!(told(&vec![warning("c"); 10]).ends_with("\nand 2 more"));
/// ```
pub fn told(warnings: &[Warning]) -> String {
    let mut lines: Vec<String> = warnings
        .iter()
        .take(WARNINGS)
        .map(ToString::to_string)
        .collect();
    if warnings.len() > WARNINGS {
        lines.push(format!("and {} more", warnings.len() - WARNINGS));
    }
    lines.join("\n")
}

/// Whether `id` is a node's or a group's id: from 1 to [`ID_BYTES`] ASCII letters, digits, `-`, `_` and `.`.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= ID_BYTES
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

/// A node's attributes as they are read, each set at most once: the first of each kept.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Attributes {
    pub label: Option<String>,
    pub shape: Option<Shape>,
    pub colour: Option<Colour>,
    pub size: Option<u8>,
    pub detail: Option<String>,
    pub reference: Option<String>,
}

impl Attributes {
    /// These, then what `other` sets that these do not.
    pub fn or(self, other: &Attributes) -> Attributes {
        Attributes {
            label: self.label.or_else(|| other.label.clone()),
            shape: self.shape.or(other.shape),
            colour: self.colour.or(other.colour),
            size: self.size.or(other.size),
            detail: self.detail.or_else(|| other.detail.clone()),
            reference: self.reference.or_else(|| other.reference.clone()),
        }
    }

    /// Set the attribute `key` to `value`, as `@{ key: value }` or the tool's JSON says it, or say why not: a key Tomori
    /// does not know, or a value it does not know, which leaves it as it was.
    pub fn set(&mut self, key: &str, value: &str) -> Result<(), String> {
        let value = value.trim();
        match key.trim().to_ascii_lowercase().as_str() {
            "label" | "title" | "name" => {
                self.label.get_or_insert_with(|| value.to_owned());
            }
            "shape" => {
                let shape = shape(value).ok_or_else(|| {
                    format!(
                        "\"{}\" is not a shape: a box, sphere, cylinder, prism, diamond or card",
                        clean(value)
                    )
                })?;
                self.shape.get_or_insert(shape);
            }
            "colour" | "color" => {
                let colour = colour(value).ok_or_else(|| {
                    format!("\"{}\" is not a colour of the palette: teal, amber, blue, violet, green, red, yellow, grey or white", clean(value))
                })?;
                self.colour.get_or_insert(colour);
            }
            "size" => {
                let size = value
                    .parse::<u8>()
                    .ok()
                    .filter(|size| (1..=SIZES).contains(size))
                    .ok_or_else(|| format!("size \"{}\" is not from 1 to {SIZES}", clean(value)))?;
                self.size.get_or_insert(size);
            }
            "detail" | "description" => {
                self.detail.get_or_insert_with(|| value.to_owned());
            }
            "ref" | "reference" => {
                self.reference.get_or_insert_with(|| value.to_owned());
            }
            other => {
                return Err(format!(
                    "\"{}\" is not an attribute of a node",
                    clean(other)
                ));
            }
        }
        Ok(())
    }
}

/// `text` as a warning quotes it: on one line, short, without double quotes.
fn clean(text: &str) -> String {
    one_line(text, LABEL_CHARS).replace('"', "'")
}

/// The shape `text` names, in any case, with Mermaid's names for the same.
pub fn shape(text: &str) -> Option<Shape> {
    match text.trim().to_ascii_lowercase().as_str() {
        "box" | "rect" | "rectangle" | "square" | "rounded" => Some(Shape::Box),
        "sphere" | "circle" | "ball" => Some(Shape::Sphere),
        "cylinder" | "cyl" | "database" | "db" => Some(Shape::Cylinder),
        "prism" | "hexagon" | "hex" => Some(Shape::Prism),
        "diamond" | "rhombus" | "decision" => Some(Shape::Diamond),
        "card" | "panel" | "subroutine" => Some(Shape::Card),
        _ => None,
    }
}

/// The colour of the palette `text` names, in any case, or `#rrggbb`.
pub fn colour(text: &str) -> Option<Colour> {
    let colour = Colour::from(text.to_owned());
    (colour != Colour::Grey || matches!(text.trim().to_ascii_lowercase().as_str(), "grey" | "gray"))
        .then_some(colour)
}

/// The style and arrow of an edge `text` names, as the tool's JSON says them.
pub fn style(text: &str) -> Option<Style> {
    match text.trim().to_ascii_lowercase().as_str() {
        "solid" => Some(Style::Solid),
        "dashed" => Some(Style::Dashed),
        "dotted" => Some(Style::Dotted),
        "thick" => Some(Style::Thick),
        "flow" => Some(Style::Flow),
        _ => None,
    }
}

pub fn arrow(text: &str) -> Option<Arrow> {
    match text.trim().to_ascii_lowercase().as_str() {
        "forward" => Some(Arrow::Forward),
        "back" => Some(Arrow::Back),
        "both" => Some(Arrow::Both),
        "none" => Some(Arrow::None),
        _ => None,
    }
}

pub fn group_kind(text: &str) -> Option<GroupKind> {
    match text.trim().to_ascii_lowercase().as_str() {
        "cluster" => Some(GroupKind::Cluster),
        "layer" => Some(GroupKind::Layer),
        "container" => Some(GroupKind::Container),
        _ => None,
    }
}

/// An edge as it is read, before it is kept: its ends by id and how it is drawn.
#[derive(Clone, Debug, PartialEq)]
pub struct EdgeDraft {
    pub from: String,
    pub to: String,
    pub label: String,
    pub direction: Arrow,
    pub style: Style,
    pub colour: Option<Colour>,
    pub weight: u8,
}

impl EdgeDraft {
    /// A solid edge from `from` to `to`, forward.
    pub fn new(from: impl Into<String>, to: impl Into<String>) -> Self {
        Self {
            from: from.into(),
            to: to.into(),
            label: String::new(),
            direction: Arrow::Forward,
            style: Style::Solid,
            colour: None,
            weight: 1,
        }
    }
}

/// What reads a diagram, from a block or from the tool's JSON, into a [`Diagram`] within its bounds: lenient by the
/// line and strict by the bound, so that a slightly wrong diagram is still shown, and its warnings say what was wrong.
#[derive(Debug, Default)]
pub struct Builder {
    layout: Layout,
    /// Each node, in the order declared: its id, its attributes, and its group, by its index, the first it was put in.
    nodes: Vec<(String, Attributes, Option<usize>)>,
    /// Each node's place among `nodes`, by its id, so that a block of thousands of mentions reads in time.
    index: std::collections::BTreeMap<String, usize>,
    /// The edges read, at most [`EDGE_DRAFTS`]; those past it are only counted, as `a & b & … --> c & d & …` writes as
    /// many as both sides' product.
    edges: Vec<EdgeDraft>,
    edges_past: usize,
    groups: Vec<Group>,
    warnings: Vec<Warning>,
    /// The line being read, from 1, for warnings, while a block is read.
    line: Option<usize>,
    /// Whether an edge naming a node never declared warns, as the tool's JSON does, which declares every node; a block,
    /// as Mermaid, declares a node by naming it, and warns only where its id is close to another's.
    strict: bool,
    /// How many nodes, edges and groups were left out past their bounds, and the first left out of each.
    over: [(usize, usize); 3],
    /// Each world reference declared, in the order declared, with the line it was declared on, and its place among them
    /// by its id.
    references: Vec<(Target, Option<usize>)>,
    reference_index: std::collections::BTreeMap<String, usize>,
    /// The world edges read, at most [`WORLD_DRAFTS`], each with its line; those past it are only counted.
    world_edges: Vec<(EdgeDraft, Option<usize>)>,
    world_past: usize,
}

/// What a world reference's line or object sets beside its kind and name: its label and its attributes, `mark` and
/// `colour`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Binding {
    pub label: Option<String>,
    pub mark: Option<Mark>,
    pub colour: Option<Colour>,
}

impl Binding {
    /// Set the attribute `key` to `value`, as `@{ key: value }` or the tool's JSON says it, or say why not, which leaves
    /// it as it was.
    pub fn set(&mut self, key: &str, value: &str) -> Result<(), String> {
        let value = value.trim();
        match key.trim().to_ascii_lowercase().as_str() {
            "label" | "title" | "name" => {
                self.label.get_or_insert_with(|| value.to_owned());
            }
            "mark" => {
                let mark = Mark::parse(value).ok_or_else(|| {
                    format!(
                        "\"{}\" is not a mark: a ring, halo, outline or none",
                        clean(value)
                    )
                })?;
                self.mark.get_or_insert(mark);
            }
            "colour" | "color" => {
                let colour = colour(value).ok_or_else(|| {
                    format!("\"{}\" is not a colour of the palette: teal, amber, blue, violet, green, red, yellow, grey or white", clean(value))
                })?;
                self.colour.get_or_insert(colour);
            }
            other => {
                return Err(format!(
                    "\"{}\" is not an attribute of a world reference: mark or colour",
                    clean(other)
                ));
            }
        }
        Ok(())
    }
}

impl Builder {
    /// A builder for a block's body, which declares a node by mentioning it.
    pub fn new() -> Self {
        Self::default()
    }

    /// A builder for the tool's JSON, which declares every node before an edge names it.
    pub fn strict() -> Self {
        Self {
            strict: true,
            ..Self::default()
        }
    }

    /// Read what follows as line `line` of a block.
    pub fn at_line(&mut self, line: usize) {
        self.line = Some(line);
    }

    /// Warn of `text`, at the line being read.
    pub fn warn(&mut self, text: impl Into<String>) {
        self.warnings.push(Warning {
            line: self.line,
            text: text.into(),
        });
    }

    pub fn set_layout(&mut self, layout: Layout) {
        self.layout = layout;
    }

    /// Bind `id` to the thing in the world `reference` names, with what `binding` sets, as `fire := object
    /// farmyard:campsite/bonfire [The bonfire]` does, so that edges to `id` are world edges. Returns whether it is bound:
    /// an id that is not valid, or that names a node already, is skipped, saying so, and a second binding of one id
    /// keeps the first.
    pub fn reference(&mut self, id: &str, reference: Reference, binding: Binding) -> bool {
        if !valid_id(id) {
            self.warn(format!(
                "\"{}\" is not a world reference's id: from 1 to {ID_BYTES} letters, digits, -, _ and .",
                clean(id)
            ));
            return false;
        }
        if self.has_node(id) {
            self.warn(format!(
                "\"{id}\" is a node already, so it is not bound to {reference}: bind a name before the edges that use it"
            ));
            return false;
        }
        if let Some(&at) = self.reference_index.get(id) {
            let known = &self.references[at].0.reference;
            if *known != reference {
                let known = known.to_string();
                self.warn(format!(
                    "\"{id}\" is bound already, to {known}; the first kept"
                ));
            }
            return true;
        }
        let mut target = Target::new(id, reference);
        target.label = binding
            .label
            .as_deref()
            .map(|label| one_line(label, crate::world::LABEL_CHARS).trim().to_owned())
            .unwrap_or_default();
        target.mark = binding.mark.unwrap_or_default();
        target.colour = binding.colour;
        self.reference_index
            .insert(id.to_owned(), self.references.len());
        self.references.push((target, self.line));
        true
    }

    /// Whether `id` is bound to a thing in the world.
    pub fn is_reference(&self, id: &str) -> bool {
        self.reference_index.contains_key(id)
    }

    /// Declare the node `id`, or add to it the attributes it does not have yet, in `group`, by its index, where it is
    /// in none yet. Returns whether it is a node: an id that is not valid, or bound to a thing in the world, is skipped,
    /// saying so where it was given attributes.
    pub fn node(&mut self, id: &str, attributes: Attributes, group: Option<usize>) -> bool {
        if !valid_id(id) {
            self.warn(format!(
                "\"{}\" is not a node's id: from 1 to {ID_BYTES} letters, digits, -, _ and .",
                clean(id)
            ));
            return false;
        }
        if self.is_reference(id) {
            if attributes != Attributes::default() {
                self.warn(format!(
                    "\"{id}\" is a world reference, not a node: give its label on its := line"
                ));
            }
            return false;
        }
        match self.index.get(id) {
            Some(&at) => {
                let (_, known, member) = &mut self.nodes[at];
                *known = std::mem::take(known).or(&attributes);
                if member.is_none() {
                    *member = group;
                }
            }
            None => {
                self.index.insert(id.to_owned(), self.nodes.len());
                self.nodes.push((id.to_owned(), attributes, group));
            }
        }
        true
    }

    /// Whether `id` names a node declared already.
    pub fn has_node(&self, id: &str) -> bool {
        self.index.contains_key(id)
    }

    /// Whether as many edges are read as are kept to choose from, so that more are only counted.
    pub fn edges_full(&self) -> bool {
        self.edges.len() >= EDGE_DRAFTS
    }

    /// Count an edge left out as the edges read are full.
    pub fn edge_past(&mut self) {
        self.edges_past += 1;
    }

    /// An edge, as `edge` says it, its ends declared where they are not yet, in `group` where they are in none.
    pub fn edge(&mut self, edge: EdgeDraft, group: Option<usize>) {
        match (self.is_reference(&edge.from), self.is_reference(&edge.to)) {
            (true, true) => {
                self.warn(format!(
                    "the edge from {} to {} is left out: an edge to a thing in the world comes from a node",
                    edge.from, edge.to
                ));
                return;
            }
            (true, false) | (false, true) => return self.world_edge(edge, group),
            (false, false) => {}
        }
        if self.edges_full() {
            self.edge_past();
            return;
        }
        for end in [&edge.from, &edge.to] {
            if self.strict && !self.has_node(end) && valid_id(end) {
                self.warn(format!(
                    "\"{}\" is not a node; made a box called {}{}",
                    clean(end),
                    end,
                    self.close(end)
                ));
            }
        }
        if self.node(&edge.from.clone(), Attributes::default(), group)
            && self.node(&edge.to.clone(), Attributes::default(), group)
        {
            self.edges.push(edge);
        }
    }

    /// A world edge, one of whose ends is bound to a thing in the world, its other declared as a node where it is not
    /// yet, in `group` where it is in none.
    fn world_edge(&mut self, edge: EdgeDraft, group: Option<usize>) {
        if self.world_edges.len() >= WORLD_DRAFTS {
            self.world_past += 1;
            return;
        }
        let node = if self.is_reference(&edge.from) {
            &edge.to
        } else {
            &edge.from
        };
        if self.strict && !self.has_node(node) && valid_id(node) {
            self.warn(format!(
                "\"{}\" is not a node; made a box called {}{}",
                clean(node),
                node,
                self.close(node)
            ));
        }
        if self.node(&node.clone(), Attributes::default(), group) {
            self.world_edges.push((edge, self.line));
        }
    }

    /// Warn where a block mentions `id`, which names no node or world reference yet, though one's id is close to it, as
    /// a misspelling.
    pub fn mentioned(&mut self, id: &str) {
        if !self.has_node(id)
            && !self.is_reference(id)
            && let Some(close) = self.closest(id)
        {
            let what = if self.is_reference(&close) {
                "a node or a world reference"
            } else {
                "a node"
            };
            self.warn(format!(
                "\"{id}\" is not {what}; made a box called {id} (did you mean {close}?)"
            ));
        }
    }

    /// ` (did you mean ID?)` for the node whose id is closest to `id`, if one is close.
    fn close(&self, id: &str) -> String {
        self.closest(id)
            .map_or(String::new(), |close| format!(" (did you mean {close}?)"))
    }

    /// The id of a node declared already within two edits of `id`, the first such written; none once the diagram has
    /// as many nodes as it keeps, since a node made then is left out, so that this costs at most [`NODES`] squared.
    fn closest(&self, id: &str) -> Option<String> {
        if self.nodes.len() >= NODES {
            return None;
        }
        self.nodes
            .iter()
            .map(|(known, ..)| known)
            .chain(self.references.iter().map(|(target, _)| &target.id))
            .find(|known| {
                // Ids that differ only in the number they end with, as leaf1 and leaf2, are not misspellings.
                let stem = |id: &str| id.trim_end_matches(|c: char| c.is_ascii_digit()).len();
                known.len().min(id.len()) > 2
                    && known.len().abs_diff(id.len()) <= 2
                    && known[..stem(known)] != id[..stem(id)]
                    && edits(known, id) <= 2
            })
            .cloned()
    }

    /// A group `id`, labelled `label`, of `kind`, in `parent`, by its index, returning its index; or, nested past
    /// [`NESTING`], its parent's, its members going there. An id that is not valid is made from its place.
    pub fn group(
        &mut self,
        id: &str,
        label: &str,
        kind: GroupKind,
        parent: Option<usize>,
    ) -> Option<usize> {
        if let Some(parent) = parent
            && self.depth(parent) >= NESTING
        {
            self.warn(format!(
                "group \"{}\" nested {} deep: its members put in \"{}\"",
                clean(id),
                NESTING + 1,
                self.groups[parent].id
            ));
            return Some(parent);
        }
        let id = if valid_id(id) {
            id.to_owned()
        } else {
            format!("group-{}", self.groups.len() + 1)
        };
        if let Some(known) = self.groups.iter().position(|group| group.id == id) {
            return Some(known);
        }
        let mut group = Group::new(id, one_line(label, LABEL_CHARS).trim());
        group.kind = kind;
        group.parent = parent.and_then(|parent| u8::try_from(parent).ok());
        self.groups.push(group);
        Some(self.groups.len() - 1)
    }

    /// Make the group at `index` of `kind`.
    pub fn group_kind(&mut self, index: usize, kind: GroupKind) {
        if let Some(group) = self.groups.get_mut(index) {
            group.kind = kind;
        }
    }

    /// How deeply the group at `index` is nested, from 1.
    fn depth(&self, index: usize) -> usize {
        let mut depth = 1;
        let mut at = self.groups[index].parent;
        while let Some(parent) = at {
            depth += 1;
            at = self.groups[usize::from(parent)].parent;
        }
        depth
    }

    /// The diagram read, within its bounds, and every warning: what was left out past a bound, counted.
    pub fn finish(mut self) -> (Diagram, Vec<Warning>) {
        self.line = None;
        // Groups past the bound are left out, their members in none, as are nodes past theirs.
        let groups: Vec<Group> = self.groups.iter().take(GROUPS).cloned().collect();
        if self.groups.len() > GROUPS {
            self.over[2] = (self.groups.len() - GROUPS, GROUPS + 1);
        }
        if self.nodes.len() > NODES {
            self.over[0] = (self.nodes.len() - NODES, NODES + 1);
        }
        let nodes: Vec<Node> = self
            .nodes
            .iter()
            .take(NODES)
            .map(|(id, attributes, group)| {
                let mut node = Node::new(id.clone(), id.clone());
                if let Some(label) = &attributes.label {
                    let label = one_line(label, LABEL_CHARS).trim().to_owned();
                    if !label.is_empty() {
                        node.label = label;
                    }
                }
                node.shape = attributes.shape.unwrap_or_default();
                node.colour = attributes.colour.unwrap_or_default();
                node.size = attributes.size.unwrap_or(1);
                node.detail = attributes
                    .detail
                    .as_deref()
                    .map(|detail| one_line(detail, DETAIL_CHARS).trim().to_owned())
                    .unwrap_or_default();
                node.reference = attributes
                    .reference
                    .as_deref()
                    .map(|reference| cut_bytes(&one_line(reference, REF_BYTES), REF_BYTES))
                    .unwrap_or_default();
                node.group = group
                    .and_then(|group| u8::try_from(group).ok())
                    .filter(|group| usize::from(*group) < groups.len());
                node
            })
            .collect();
        let index = |id: &str| nodes.iter().position(|node| node.id == id);
        let mut edges: Vec<Edge> = Vec::new();
        let mut warnings = std::mem::take(&mut self.warnings);
        let mut pairs = 0;
        let mut kept = 0;
        for draft in &self.edges {
            let (Some(from), Some(to)) = (index(&draft.from), index(&draft.to)) else {
                continue;
            };
            if from == to {
                warnings.push(Warning {
                    line: None,
                    text: format!("the edge from {} to itself is left out", draft.from),
                });
                continue;
            }
            let between = edges
                .iter()
                .filter(|edge| {
                    (usize::from(edge.from), usize::from(edge.to)) == (from, to)
                        || (usize::from(edge.from), usize::from(edge.to)) == (to, from)
                })
                .count();
            if between >= PAIR_EDGES {
                pairs += 1;
                continue;
            }
            kept += 1;
            if edges.len() == EDGES {
                if self.over[1].0 == 0 {
                    self.over[1].1 = kept;
                }
                self.over[1].0 += 1;
                continue;
            }
            // Both fit in a u16, as at most [`NODES`] are kept.
            let mut edge = Edge::new(from as u16, to as u16);
            edge.label = one_line(&draft.label, EDGE_LABEL_CHARS).trim().to_owned();
            edge.direction = draft.direction;
            edge.style = draft.style;
            edge.colour = draft.colour;
            edge.weight = draft.weight.clamp(1, WEIGHTS);
            edges.push(edge);
        }
        // Those past what is read to choose from are left out past the bound.
        if self.edges_past > 0 {
            if self.over[1].0 == 0 {
                self.over[1].1 = kept + 1;
            }
            self.over[1].0 += self.edges_past;
        }
        if pairs > 0 {
            warnings.push(Warning {
                line: None,
                text: format!(
                    "{pairs} edges left out: at most {PAIR_EDGES} between one pair of nodes"
                ),
            });
        }
        for ((count, first), (kind, most)) in
            self.over
                .iter()
                .zip([("nodes", NODES), ("edges", EDGES), ("groups", GROUPS)])
        {
            if *count > 0 {
                warnings.push(Warning {
                    line: None,
                    text: format!(
                        "{kind} {first} to {} left out: at most {most}",
                        first + count - 1
                    ),
                });
            }
        }
        let (world, world_edges) = self.world(&nodes, &mut warnings);
        (
            Diagram {
                layout: self.layout,
                nodes,
                edges,
                groups,
                extent: [0; 3],
                world,
                world_edges,
            },
            warnings,
        )
    }

    /// The world references kept, at most [`REFERENCES`], and the world edges between `nodes` and them, at most
    /// [`WORLD_EDGES`] and [`REFERENCE_EDGES`] to any one, each pointing at its reference where its arrow did, warning in
    /// `warnings` of what was left out and of each reference no edge uses.
    fn world(&mut self, nodes: &[Node], warnings: &mut Vec<Warning>) -> (Vec<Target>, Vec<Edge>) {
        let kept = self.references.len().min(REFERENCES);
        if self.references.len() > REFERENCES {
            warnings.push(Warning {
                line: None,
                text: format!(
                    "world references {} to {} left out, with their edges: at most {REFERENCES}",
                    REFERENCES + 1,
                    self.references.len()
                ),
            });
        }
        let mut edges: Vec<Edge> = Vec::new();
        let (mut past, mut crowded) = (self.world_past, 0);
        for (draft, _) in &self.world_edges {
            // Its node, its reference, and whether it was written from the reference.
            let (node, reference, back) = match self.reference_index.get(&draft.from) {
                Some(&reference) => (&draft.to, reference, true),
                None => (&draft.from, self.reference_index[&draft.to], false),
            };
            let Some(node) = nodes.iter().position(|kept| kept.id == *node) else {
                continue;
            };
            if reference >= kept {
                continue;
            }
            if edges.len() == WORLD_EDGES {
                past += 1;
                continue;
            }
            if edges
                .iter()
                .filter(|edge| usize::from(edge.to) == reference)
                .count()
                >= REFERENCE_EDGES
            {
                crowded += 1;
                continue;
            }
            // Both fit in a u16, as at most [`NODES`] and [`REFERENCES`] are kept.
            let mut edge = Edge::new(node as u16, reference as u16);
            edge.label = one_line(&draft.label, EDGE_LABEL_CHARS).trim().to_owned();
            edge.direction = match (back, draft.direction) {
                (true, Arrow::Forward) => Arrow::Back,
                (true, Arrow::Back) => Arrow::Forward,
                (_, direction) => direction,
            };
            edge.style = draft.style;
            edge.colour = draft.colour;
            edge.weight = draft.weight.clamp(1, WEIGHTS);
            edges.push(edge);
        }
        if past > 0 {
            warnings.push(Warning {
                line: None,
                text: format!("{past} world edges left out: at most {WORLD_EDGES}"),
            });
        }
        if crowded > 0 {
            warnings.push(Warning {
                line: None,
                text: format!(
                    "{crowded} world edges left out: at most {REFERENCE_EDGES} to one world reference"
                ),
            });
        }
        let world: Vec<Target> = self
            .references
            .iter()
            .take(kept)
            .enumerate()
            .map(|(index, (target, line))| {
                if !edges.iter().any(|edge| usize::from(edge.to) == index) {
                    warnings.push(Warning {
                        line: *line,
                        text: format!(
                            "\"{}\" is bound to {} but no edge uses it, so nothing points at it",
                            target.id, target.reference
                        ),
                    });
                }
                target.clone()
            })
            .collect();
        (world, edges)
    }
}

/// How many single characters must be inserted, removed or changed to make `a` into `b`, by which a misspelt name is
/// matched to the one meant.
///
/// ```
/// use tomori_protocol::diagram::edits;
///
/// assert_eq!(edits("stik", "stick"), 1);
/// assert_eq!(edits("bonfire", "bonfire"), 0);
/// ```
pub fn edits(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut previous = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let here = row[j + 1];
            row[j + 1] = (previous + usize::from(ca != *cb))
                .min(row[j] + 1)
                .min(here + 1);
            previous = here;
        }
    }
    row[b.len()]
}

impl Diagram {
    /// The diagram the `show_diagram` tool's JSON describes, its `layout`, `nodes`, `edges` and `groups`, each object
    /// with the fields `design/diagrams.md` lists, read leniently as a block is, and every warning; or why not, where
    /// one of them is not of the shape the tool takes.
    pub fn from_json(value: &serde_json::Value) -> Result<(Diagram, Vec<Warning>), String> {
        let mut builder = Builder::strict();
        let text = |value: &serde_json::Value, key: &str| -> Option<String> {
            value.get(key).and_then(|value| match value {
                serde_json::Value::String(text) => Some(text.clone()),
                serde_json::Value::Number(number) => Some(number.to_string()),
                _ => None,
            })
        };
        let list = |key: &str| -> Result<Vec<serde_json::Value>, String> {
            match value.get(key) {
                None | Some(serde_json::Value::Null) => Ok(Vec::new()),
                Some(serde_json::Value::Array(items)) => Ok(items.clone()),
                Some(_) => Err(format!("{key} must be a list.")),
            }
        };
        match value.get("layout") {
            None | Some(serde_json::Value::Null) => {}
            Some(serde_json::Value::String(layout)) => layout_or_warn(&mut builder, layout),
            Some(layout) => {
                let kind = text(layout, "kind").unwrap_or_default();
                let extra = text(layout, "direction")
                    .or_else(|| text(layout, "hub"))
                    .unwrap_or_default();
                layout_or_warn(&mut builder, &format!("{kind} {extra}"));
            }
        }
        let groups = list("groups")?;
        for group in &groups {
            let id = text(group, "id").unwrap_or_default();
            let kind = text(group, "kind").map_or(GroupKind::Cluster, |kind| {
                group_kind(&kind).unwrap_or_else(|| {
                    builder.warn(format!(
                        "\"{}\" is not a kind of group: a cluster, layer or container",
                        clean(&kind)
                    ));
                    GroupKind::Cluster
                })
            });
            let parent = text(group, "parent")
                .and_then(|parent| builder.groups.iter().position(|group| group.id == parent));
            builder.group(
                &id,
                &text(group, "label").unwrap_or_else(|| id.clone()),
                kind,
                parent,
            );
        }
        for thing in list("world")? {
            let id = text(&thing, "id").unwrap_or_default();
            let kind = text(&thing, "kind").map_or(Kind::Unknown, |kind| {
                Kind::parse(&kind).unwrap_or_else(|| {
                    builder.warn(format!(
                        "world reference {}: \"{}\" is not a kind: an object, anchor, wall, panel, worker, person or agent; taken as whatever the name names",
                        clean(&id),
                        clean(&kind)
                    ));
                    Kind::Unknown
                })
            });
            let name = text(&thing, "name").unwrap_or_default();
            if name.trim().is_empty() && kind != Kind::Agent {
                builder.warn(format!(
                    "world reference {} is left out: it names nothing",
                    clean(&id)
                ));
                continue;
            }
            let mut binding = Binding::default();
            for key in ["label", "mark", "colour", "color"] {
                if let Some(value) = text(&thing, key)
                    && let Err(why) = binding.set(key, &value)
                {
                    builder.warn(format!("world reference {}: {why}", clean(&id)));
                }
            }
            builder.reference(&id, Reference::new(kind, &name), binding);
        }
        for node in list("nodes")? {
            let id = text(&node, "id").unwrap_or_default();
            let mut attributes = Attributes::default();
            for key in ["label", "shape", "colour", "color", "size", "detail", "ref"] {
                if let Some(value) = text(&node, key)
                    && let Err(why) = attributes.set(key, &value)
                {
                    builder.warn(format!("node {}: {why}", clean(&id)));
                }
            }
            let group = text(&node, "group").and_then(|group| {
                let found = builder.groups.iter().position(|known| known.id == group);
                if found.is_none() {
                    builder.warn(format!(
                        "node {}: \"{}\" is not a group",
                        clean(&id),
                        clean(&group)
                    ));
                }
                found
            });
            builder.node(&id, attributes, group);
        }
        for edge in list("edges")? {
            let mut draft = EdgeDraft::new(
                text(&edge, "from").unwrap_or_default(),
                text(&edge, "to").unwrap_or_default(),
            );
            draft.label = text(&edge, "label").unwrap_or_default();
            if let Some(value) = text(&edge, "style") {
                match style(&value) {
                    Some(style) => draft.style = style,
                    None => builder.warn(format!(
                        "\"{}\" is not a style of edge: solid, dashed, dotted, thick or flow",
                        clean(&value)
                    )),
                }
            }
            if let Some(value) = text(&edge, "direction") {
                match arrow(&value) {
                    Some(direction) => draft.direction = direction,
                    None => builder.warn(format!(
                        "\"{}\" is not a direction of edge: forward, back, both or none",
                        clean(&value)
                    )),
                }
            }
            draft.colour = text(&edge, "colour")
                .or_else(|| text(&edge, "color"))
                .and_then(|value| colour(&value));
            draft.weight = text(&edge, "weight")
                .and_then(|weight| weight.parse().ok())
                .unwrap_or(1);
            if !valid_id(&draft.from) || !valid_id(&draft.to) {
                builder.warn(format!(
                    "the edge from \"{}\" to \"{}\" is left out: each end must be a node's id",
                    clean(&draft.from),
                    clean(&draft.to)
                ));
                continue;
            }
            builder.edge(draft, None);
        }
        Ok(builder.finish())
    }

    /// The text outline of it, which a client that draws no volumes shows as the panel's body: its nodes in no group,
    /// then each group as a heading over its nodes, each a list item with its detail, and its edges, a line each, as
    /// `Renderer → Octree: queries`.
    ///
    /// ```
    /// use tomori_protocol::diagram::parse;
    ///
    /// let (diagram, _) = parse("a[Renderer] --> b[Octree] : queries\nsubgraph s [Scene]\n  c\nend\n");
    /// assert_eq!(diagram.outline(), "- Renderer\n- Octree\n\n**Scene**\n- c\n\n**Edges**\n- Renderer → Octree: queries");
    /// ```
    pub fn outline(&self) -> String {
        let item = |node: &Node| {
            if node.detail.is_empty() {
                format!("- {}", node.label)
            } else {
                format!("- {}: {}", node.label, node.detail)
            }
        };
        let mut sections: Vec<String> = Vec::new();
        let loose: Vec<String> = self
            .nodes
            .iter()
            .filter(|node| node.group.is_none())
            .map(item)
            .collect();
        if !loose.is_empty() {
            sections.push(loose.join("\n"));
        }
        for (index, group) in self.groups.iter().enumerate() {
            let members: Vec<String> = self
                .nodes
                .iter()
                .filter(|node| node.group.map(usize::from) == Some(index))
                .map(item)
                .collect();
            let heading = if group.label.is_empty() {
                &group.id
            } else {
                &group.label
            };
            sections.push(
                std::iter::once(format!("**{heading}**"))
                    .chain(members)
                    .collect::<Vec<_>>()
                    .join("\n"),
            );
        }
        let edges: Vec<String> = self
            .edges
            .iter()
            .filter_map(|edge| {
                let (from, to) = (
                    self.nodes.get(usize::from(edge.from))?,
                    self.nodes.get(usize::from(edge.to))?,
                );
                let arrow = match edge.direction {
                    Arrow::Forward => "→",
                    Arrow::Back => "←",
                    Arrow::Both => "↔",
                    Arrow::None => "—",
                };
                Some(if edge.label.is_empty() {
                    format!("- {} {arrow} {}", from.label, to.label)
                } else {
                    format!("- {} {arrow} {}: {}", from.label, to.label, edge.label)
                })
            })
            .chain(
                (0..self.world_edges.len())
                    .filter_map(|index| self.world_edge_name(index, true))
                    .map(|name| format!("- {name}")),
            )
            .collect();
        if !edges.is_empty() {
            sections.push(format!("**Edges**\n{}", edges.join("\n")));
        }
        sections.join("\n\n")
    }

    /// It as a client draws it, whatever a server sent: at most [`NODES`], [`EDGES`] and [`GROUPS`], every edge between
    /// two of its nodes with at most [`WAYPOINTS`], every group's parent an earlier group and every node's group one
    /// of them, sizes and weights within their bounds, every label within its length and the extent within
    /// [`EXTENT_MM`].
    pub fn bounded(&self) -> Diagram {
        let mut diagram = self.clone();
        diagram.nodes.truncate(NODES);
        diagram.groups.truncate(GROUPS);
        let groups = diagram.groups.len();
        for (index, group) in diagram.groups.iter_mut().enumerate() {
            group.parent = group.parent.filter(|parent| usize::from(*parent) < index);
            group.label = one_line(&group.label, LABEL_CHARS);
        }
        for node in &mut diagram.nodes {
            node.size = node.size.clamp(1, SIZES);
            node.group = node.group.filter(|group| usize::from(*group) < groups);
            node.label = one_line(&node.label, LABEL_CHARS);
            node.detail = one_line(&node.detail, DETAIL_CHARS);
        }
        let nodes = diagram.nodes.len();
        diagram.edges.retain(|edge| {
            usize::from(edge.from) < nodes && usize::from(edge.to) < nodes && edge.from != edge.to
        });
        diagram.edges.truncate(EDGES);
        for edge in &mut diagram.edges {
            edge.via.truncate(WAYPOINTS);
            edge.weight = edge.weight.clamp(1, WEIGHTS);
            edge.label = one_line(&edge.label, EDGE_LABEL_CHARS);
        }
        diagram.extent = diagram.extent.map(|side| side.min(EXTENT_MM));
        diagram.world.truncate(REFERENCES);
        diagram.world = diagram.world.iter().map(Target::bounded).collect();
        let world = diagram.world.len();
        diagram
            .world_edges
            .retain(|edge| usize::from(edge.from) < nodes && usize::from(edge.to) < world);
        diagram.world_edges.truncate(WORLD_EDGES);
        for edge in &mut diagram.world_edges {
            edge.via.clear();
            edge.weight = edge.weight.clamp(1, WEIGHTS);
            edge.label = one_line(&edge.label, EDGE_LABEL_CHARS);
        }
        diagram
    }

    /// The world edge at `index`, as it is named: `"1. Find the fire → The bonfire: here"`, its node's label, an arrow
    /// its way, its target's label and its label, and, `with_reference`, what the target refers to after its label, as
    /// `The bonfire (farmyard:campsite/bonfire)`, as the outline lists it.
    ///
    /// ```
    /// use tomori_protocol::diagram::parse;
    ///
    /// let (diagram, _) = parse("fire := object farmyard:campsite/bonfire [The bonfire]\nstep[1. Find the fire] --> fire : here\n");
    /// assert_eq!(diagram.world_edge_name(0, false).unwrap(), "1. Find the fire → The bonfire: here");
    /// assert_eq!(
    ///     diagram.world_edge_name(0, true).unwrap(),
    ///     "1. Find the fire → The bonfire (farmyard:campsite/bonfire): here"
    /// );
    /// ```
    pub fn world_edge_name(&self, index: usize, with_reference: bool) -> Option<String> {
        let edge = self.world_edges.get(index)?;
        let node = self.nodes.get(usize::from(edge.from))?;
        let target = self.world.get(usize::from(edge.to))?;
        let name = &target.reference.name;
        let label = if !target.label.is_empty() {
            target.label.as_str()
        } else if !name.is_empty() {
            name.as_str()
        } else {
            target.reference.kind.name()
        };
        let arrow = match edge.direction {
            Arrow::Forward => "→",
            Arrow::Back => "←",
            Arrow::Both => "↔",
            Arrow::None => "—",
        };
        let reference = if with_reference && !name.is_empty() && name != label {
            format!(" ({name})")
        } else {
            String::new()
        };
        let suffix = if edge.label.is_empty() {
            String::new()
        } else {
            format!(": {}", edge.label)
        };
        Some(format!("{} {arrow} {label}{reference}{suffix}", node.label))
    }

    /// The index of the node `id`, if it has one.
    pub fn node(&self, id: &str) -> Option<usize> {
        self.nodes.iter().position(|node| node.id == id)
    }

    /// The edge at `index`, as it is named: `"Frame step → projection::faces: panels changed"`, its ends' labels, an
    /// arrow its way and its label.
    pub fn edge_name(&self, index: usize) -> Option<String> {
        let edge = self.edges.get(index)?;
        let (from, to) = (
            self.nodes.get(usize::from(edge.from))?,
            self.nodes.get(usize::from(edge.to))?,
        );
        let arrow = if edge.direction == Arrow::Back {
            "←"
        } else {
            "→"
        };
        Some(if edge.label.is_empty() {
            format!("{} {arrow} {}", from.label, to.label)
        } else {
            format!("{} {arrow} {}: {}", from.label, to.label, edge.label)
        })
    }
}

/// Set `builder`'s layout to the one `text` names, or warn and keep layered down.
fn layout_or_warn(builder: &mut Builder, text: &str) {
    match Layout::parse(text) {
        Some(layout) => builder.set_layout(layout),
        None => builder.warn(format!(
            "\"{}\" is not a layout: layered with down, up, right, left or away, radial, force or grid; laid out layered down",
            clean(text)
        )),
    }
}

#[cfg(test)]
mod tests;
