// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Volumes: panels with depth, each showing the diagram the server laid out, its nodes as lit solids, its edges as
//! tubes through their waypoints with cones for arrowheads, its groups as trays and frames, and its labels from the
//! shared glyph atlas, each turned to face the viewer about the vertical. Its face is a panel's, its title above a plate
//! the diagram stands in front of: a dark one in a wall's slot, as a shallow display case, and a faint one floating, so
//! that what is behind it shows between its nodes. What it draws is built once for each version of its diagram and
//! kept, as a panel's face is; a frame moves it by its panel's model alone, and dims what is not the node the
//! participant focuses on and its neighbours. The ray meets its nodes and edges, nearest first, which name them to the
//! agent. See `design/diagrams.md`.
use std::sync::Arc;

use glam::{Mat3, Mat4, Vec3};
use tomori_protocol::Panel;
use tomori_protocol::diagram::{Arrow, Diagram, GroupKind, Shape, Style};

use super::{BRIGHT, DIM, FIT_ROWS, Fit, PADDING, PIXEL, TABLE_COLUMNS};
use crate::canvas::Canvas;
use crate::controller_ui::Sheet;
use crate::pointing::Ray;
use crate::shapes::Mark;
use crate::ui_text::{CELL_HEIGHT, CELL_WIDTH, Row, Span, Style as TextStyle, Wrap, wrap};

/// The shortest and longest a floating volume's longest side is, in metres, at its own scale of 1.
pub const FLOATING: (f32, f32) = (0.4, 1.2);

/// The most labels drawn of one volume's nodes, the largest first, and of its edges.
pub const NODE_LABELS: usize = 32;
pub const EDGE_LABELS: usize = 16;

/// A label's font pixel, in metres, for a volume drawn at its own scale; one drawn smaller draws them smaller, to at
/// least [`LEAST_LABEL`] of it.
const LABEL_PIXEL: f32 = 0.0013;
const LEAST_LABEL: f32 = 0.6;

/// How far a volume's back stands in front of its face, in metres, so that its plate is behind everything it draws.
const STAND_OFF: f32 = 0.01;

/// The radius of an edge's tube, of its arrowhead's cone and how long that is, and a flow's beads, in millimetres of
/// the volume.
const TUBE_MM: f32 = 4.0;
const CONE_MM: (f32, f32) = (12.0, 30.0);
const BEAD_MM: f32 = 9.0;
/// A dashed edge's dashes and gaps, and a dotted one's spacing, in millimetres, and the most of either along an edge.
const DASH_MM: (f32, f32) = (24.0, 16.0);
const DOT_MM: f32 = 22.0;
const DASHES: usize = 24;
/// How many beads move along a flow, and how fast, in millimetres a second.
const BEADS: usize = 3;
const FLOW_MM: f32 = 120.0;

/// How bright what the participant does not focus on is drawn, as a share.
const DIMMED: f32 = 0.2;
/// An edge's colour, unless it has one, and a group's tray's, frame's and floor's.
const EDGE: [f32; 3] = [0.6, 0.66, 0.74];
const TRAY: [f32; 3] = [0.16, 0.2, 0.26];
const FRAME: [f32; 3] = [0.34, 0.4, 0.5];

/// How far the ray may pass an edge and still meet it: 1.5 cm, or half a degree, whichever is larger.
const EDGE_REACH: (f32, f32) = (0.015, 0.0087);

/// The unit solids a volume is drawn with, each one mesh drawn for every instance of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Solid {
    Box,
    Sphere,
    Cylinder,
    /// A hexagonal prism, along its height.
    Prism,
    /// An octahedron, its points up, down and to each side.
    Diamond,
    /// A cone, its point up.
    Cone,
    /// A thin tube, along its height, as an edge's segment.
    Tube,
}

impl Solid {
    /// Every solid, in the order the renderer keeps their meshes.
    pub const ALL: [Solid; 7] = [
        Solid::Box,
        Solid::Sphere,
        Solid::Cylinder,
        Solid::Prism,
        Solid::Diamond,
        Solid::Cone,
        Solid::Tube,
    ];
}

/// One solid of a volume as a frame draws it, in the scene: where its middle is, how large it is along each of its
/// axes, which way they point, and its colour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Instance {
    pub solid: Solid,
    pub position: Vec3,
    pub scale: Vec3,
    pub basis: Mat3,
    pub colour: [f32; 3],
}

/// What a solid of the volume stands for, which the participant's focus dims or not.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Owner {
    Node(usize),
    Edge(usize),
    Group,
}

/// A solid of a volume, in its own millimetres.
#[derive(Clone, Debug, PartialEq)]
struct Part {
    solid: Solid,
    position: Vec3,
    scale: Vec3,
    basis: Mat3,
    colour: [f32; 3],
    owner: Owner,
}

/// A label of a volume, its canvas and where it stands, in the volume's own millimetres.
#[derive(Clone, Debug)]
struct Label {
    canvas: Arc<Canvas>,
    at: Vec3,
    owner: Owner,
}

/// What a volume draws, built once from its diagram: its solids and labels in its own millimetres, where its nodes
/// and edges are, which the ray meets, and which edges move.
#[derive(Debug)]
pub struct Mesh {
    /// The diagram as it was sent, by which it is kept, and as it is drawn, within its bounds.
    source: Diagram,
    diagram: Diagram,
    parts: Vec<Part>,
    labels: Vec<Label>,
    /// Each node's middle and radius.
    nodes: Vec<(Vec3, f32)>,
    /// Each edge's path, from surface to surface, and its colour.
    edges: Vec<(Vec<Vec3>, [f32; 3])>,
    /// The edges that flow.
    flows: Vec<usize>,
    /// Each node's detail card, for a node with a detail, which the participant focusing on it sees beside it.
    cards: Vec<Option<Arc<Canvas>>>,
}

/// A node's or an edge's colour, as the palette has it.
fn rgb(colour: tomori_protocol::chart::Colour) -> [f32; 3] {
    colour.rgb()
}

/// The basis turning a solid's height, its Y, along `direction`.
fn along(direction: Vec3) -> Mat3 {
    Mat3::from_quat(glam::Quat::from_rotation_arc(
        Vec3::Y,
        direction.try_normalize().unwrap_or(Vec3::Y),
    ))
}

impl Mesh {
    /// What `diagram` draws, as [`Diagram::bounded`] holds it, whatever a server sent.
    pub fn new(source: &Diagram) -> Self {
        let diagram = source.bounded();
        let point = |at: [i16; 3]| Vec3::new(f32::from(at[0]), f32::from(at[1]), f32::from(at[2]));
        let mut parts = Vec::new();
        let mut labels = Vec::new();
        let nodes: Vec<(Vec3, f32)> = diagram
            .nodes
            .iter()
            .map(|node| (point(node.at), node.width() / 2.0))
            .collect();
        for (index, node) in diagram.nodes.iter().enumerate() {
            let (at, radius) = nodes[index];
            let width = radius * 2.0;
            let (solid, scale) = match node.shape {
                Shape::Sphere => (Solid::Sphere, Vec3::splat(width)),
                Shape::Cylinder => (Solid::Cylinder, Vec3::new(width, width * 0.8, width)),
                Shape::Prism => (Solid::Prism, Vec3::new(width, width * 0.7, width)),
                Shape::Diamond => (Solid::Diamond, Vec3::splat(width)),
                Shape::Card => (Solid::Box, Vec3::new(width * 1.4, width, width * 0.12)),
                _ => (Solid::Box, Vec3::new(width, width * 0.7, width * 0.7)),
            };
            parts.push(Part {
                solid,
                position: at,
                scale,
                basis: Mat3::IDENTITY,
                colour: rgb(node.colour),
                owner: Owner::Node(index),
            });
        }
        let mut edges = Vec::new();
        let mut flows = Vec::new();
        for (index, edge) in diagram.edges.iter().enumerate() {
            let (from, to) = (nodes[usize::from(edge.from)], nodes[usize::from(edge.to)]);
            let mut path: Vec<Vec3> = std::iter::once(from.0)
                .chain(edge.via.iter().copied().map(point))
                .chain(std::iter::once(to.0))
                .collect();
            // From surface to surface.
            let last = path.len() - 1;
            let start = (path[1] - path[0]).try_normalize().unwrap_or(Vec3::X);
            let end = (path[last] - path[last - 1])
                .try_normalize()
                .unwrap_or(Vec3::X);
            path[0] += start * from.1;
            path[last] -= end * to.1;
            let colour = edge.colour.map_or(EDGE, rgb);
            let radius = TUBE_MM
                * (1.0 + 0.5 * f32::from(edge.weight.saturating_sub(1)))
                * if edge.style == Style::Thick { 2.0 } else { 1.0 };
            let owner = Owner::Edge(index);
            tubes(&path, edge.style, radius, colour, owner, &mut parts);
            let (head, tail) = match edge.direction {
                Arrow::Forward => (true, false),
                Arrow::Back => (false, true),
                Arrow::Both => (true, true),
                _ => (false, false),
            };
            for (wanted, tip, direction) in [(head, path[last], end), (tail, path[0], -start)] {
                if wanted {
                    parts.push(Part {
                        solid: Solid::Cone,
                        position: tip - direction * CONE_MM.1 / 2.0,
                        scale: Vec3::new(CONE_MM.0 * 2.0, CONE_MM.1, CONE_MM.0 * 2.0),
                        basis: along(direction),
                        colour,
                        owner,
                    });
                }
            }
            if edge.style == Style::Flow {
                flows.push(index);
            }
            edges.push((path, colour));
        }
        for group in &diagram.groups {
            let (low, high) = (point(group.low), point(group.high));
            let size = (high - low).max(Vec3::ONE);
            let middle = (low + high) / 2.0;
            match group.kind {
                GroupKind::Container => {
                    // Its twelve edges, as a frame.
                    for axis in 0..3 {
                        let mut scale = Vec3::splat(3.0);
                        scale[axis] = size[axis];
                        let (a, b) = ((axis + 1) % 3, (axis + 2) % 3);
                        for corner in 0..4 {
                            let mut position = middle;
                            position[a] = if corner & 1 == 0 { low[a] } else { high[a] };
                            position[b] = if corner & 2 == 0 { low[b] } else { high[b] };
                            parts.push(Part {
                                solid: Solid::Box,
                                position,
                                scale,
                                basis: Mat3::IDENTITY,
                                colour: FRAME,
                                owner: Owner::Group,
                            });
                        }
                    }
                }
                _ => parts.push(Part {
                    // A tray beneath its members, or a floor, thicker.
                    solid: Solid::Box,
                    position: Vec3::new(middle.x, low.y, middle.z),
                    scale: Vec3::new(
                        size.x,
                        if group.kind == GroupKind::Layer {
                            10.0
                        } else {
                            5.0
                        },
                        size.z,
                    ),
                    basis: Mat3::IDENTITY,
                    colour: TRAY,
                    owner: Owner::Group,
                }),
            }
            let text = if group.label.is_empty() {
                &group.id
            } else {
                &group.label
            };
            labels.push(Label {
                canvas: Arc::new(label(text, None, DIM, 0.6)),
                at: Vec3::new(low.x, high.y, high.z),
                owner: Owner::Group,
            });
        }
        // The largest nodes' labels first, in the order written, then the edges' with labels.
        let mut order: Vec<usize> = (0..diagram.nodes.len()).collect();
        order.sort_by_key(|index| std::cmp::Reverse(diagram.nodes[*index].size));
        for index in order.into_iter().take(NODE_LABELS) {
            let node = &diagram.nodes[index];
            let (at, radius) = nodes[index];
            let detail = (node.shape == Shape::Card && !node.detail.is_empty())
                .then_some(node.detail.as_str());
            labels.push(Label {
                canvas: Arc::new(label(&node.label, detail, BRIGHT, 0.85)),
                at: at + Vec3::Y * (radius + 18.0),
                owner: Owner::Node(index),
            });
        }
        for (index, edge) in diagram
            .edges
            .iter()
            .enumerate()
            .filter(|(_, edge)| !edge.label.is_empty())
            .take(EDGE_LABELS)
        {
            let path = &edges[index].0;
            labels.push(Label {
                canvas: Arc::new(label(&edge.label, None, DIM, 0.7)),
                at: middle_of(path) + Vec3::Y * 12.0,
                owner: Owner::Edge(index),
            });
        }
        let cards = diagram
            .nodes
            .iter()
            .map(|node| {
                (!node.detail.is_empty())
                    .then(|| Arc::new(label(&node.label, Some(&node.detail), BRIGHT, 0.92)))
            })
            .collect();
        Self {
            cards,
            source: source.clone(),
            diagram,
            parts,
            labels,
            nodes,
            edges,
            flows,
        }
    }

    /// The diagram it was built from, as it was sent.
    pub fn source(&self) -> &Diagram {
        &self.source
    }

    /// The diagram it draws, as it holds it.
    pub fn diagram(&self) -> &Diagram {
        &self.diagram
    }

    /// How many solids it draws, flows' beads besides.
    pub fn solids(&self) -> usize {
        self.parts.len() + self.flows.len() * BEADS
    }

    /// How many labels it draws.
    pub fn labels(&self) -> usize {
        self.labels.len()
    }

    /// The node or edge `ray` meets first, in the volume drawn with `model`, from its own millimetres into the scene,
    /// and how far along the ray: a node by its bounds, an edge within [`EDGE_REACH`] of its path.
    pub fn hit(&self, model: Mat4, ray: &Ray) -> Option<(Met, f32)> {
        let inverse = model.inverse();
        let origin = inverse.transform_point3(ray.origin);
        // Along which the distance in the scene is the same as along the ray.
        let direction = inverse.transform_vector3(ray.direction);
        let per_metre = direction.length();
        let mut best: Option<(Met, f32)> = None;
        let mut take = |part: Met, distance: f32| {
            if distance >= 0.0 && best.as_ref().is_none_or(|(_, nearest)| distance < *nearest) {
                best = Some((part, distance));
            }
        };
        for (index, (centre, radius)) in self.nodes.iter().enumerate() {
            // Where the ray passes nearest the node's middle, and whether within its radius.
            let to = *centre - origin;
            let along = to.dot(direction) / direction.length_squared();
            let nearest = origin + direction * along;
            let off = (nearest - *centre).length_squared();
            let reach = radius * 1.1;
            if off <= reach * reach {
                let back = ((reach * reach - off).sqrt()) / per_metre;
                take(Met::Node(index), along - back);
            }
        }
        for (index, (path, _)) in self.edges.iter().enumerate() {
            for pair in path.windows(2) {
                let (distance, apart) = passing(origin, direction, pair[0], pair[1]);
                let reach = EDGE_REACH.0.max(EDGE_REACH.1 * distance) * per_metre;
                if apart <= reach {
                    take(Met::Edge(index), distance);
                }
            }
        }
        best
    }

    /// The labels it draws with `model`, each turned about the vertical to face `viewer`, sized as the volume is drawn,
    /// what the participant does not focus on, `focus`, dimmed.
    pub fn sheets(&self, model: Mat4, viewer: Vec3, focus: Option<usize>, sheets: &mut Vec<Sheet>) {
        let scale = label_scale(model);
        for label in &self.labels {
            if focus.is_some() && !self.lit(label.owner, focus) {
                continue;
            }
            let at = model.transform_point3(label.at);
            sheets.push(Sheet {
                canvas: Some(label.canvas.clone()),
                model: Some(crate::recording::facing(
                    at,
                    Vec3::new(viewer.x, at.y, viewer.z),
                    scale,
                )),
            });
        }
    }

    /// Whether what `owner` stands for is lit while the participant focuses on the node `focus`: it, the nodes it is
    /// joined to and the edges between them; everything, without a focus.
    fn lit(&self, owner: Owner, focus: Option<usize>) -> bool {
        let Some(focus) = focus else {
            return true;
        };
        let joined = |edge: &tomori_protocol::diagram::Edge| {
            usize::from(edge.from) == focus || usize::from(edge.to) == focus
        };
        match owner {
            Owner::Node(node) => {
                node == focus
                    || self.diagram.edges.iter().any(|edge| {
                        joined(edge)
                            && (usize::from(edge.from) == node || usize::from(edge.to) == node)
                    })
            }
            Owner::Edge(edge) => joined(&self.diagram.edges[edge]),
            Owner::Group => false,
        }
    }

    /// The detail card of the node `node`, as the participant focusing on it sees it beside it, drawn with `model`, facing
    /// `viewer`: its label and its detail, or `None` for a node without a detail.
    pub fn card(&self, node: usize, model: Mat4, viewer: Vec3) -> Option<Sheet> {
        let canvas = self.cards.get(node)?.clone()?;
        let (at, radius) = self.nodes[node];
        let at = model.transform_point3(at + Vec3::X * (radius + 20.0));
        let scale = label_scale(model);
        // Its left edge beside the node.
        let (width, _) = canvas.size();
        let model = crate::recording::facing(at, Vec3::new(viewer.x, at.y, viewer.z), scale)
            * Mat4::from_translation(Vec3::X * width / 2.0);
        Some(Sheet {
            canvas: Some(canvas),
            model: Some(model),
        })
    }

    /// Its solids drawn with `model`, from its own millimetres into the scene, `time` seconds into the flows' movement,
    /// what the participant does not focus on, `focus`, dimmed: each a unit solid moved, turned and scaled.
    pub fn instances(&self, model: Mat4, focus: Option<usize>, time: f32, out: &mut Vec<Instance>) {
        let (scale, rotation, translation) = model.to_scale_rotation_translation();
        let turn = Mat3::from_quat(rotation);
        let mut push =
            |solid: Solid, position: Vec3, size: Vec3, basis: Mat3, colour: [f32; 3], lit: bool| {
                out.push(Instance {
                    solid,
                    position: rotation * (position * scale) + translation,
                    scale: size * scale,
                    basis: turn * basis,
                    colour: if lit {
                        colour
                    } else {
                        colour.map(|channel| channel * DIMMED)
                    },
                });
            };
        for part in &self.parts {
            push(
                part.solid,
                part.position,
                part.scale,
                part.basis,
                part.colour,
                self.lit(part.owner, focus),
            );
        }
        for &edge in &self.flows {
            let (path, colour) = &self.edges[edge];
            let length = path_length(path);
            let bright = colour.map(|channel| (channel * 1.6).min(1.0));
            for bead in 0..BEADS {
                let share = (time * FLOW_MM / length.max(1.0) + bead as f32 / BEADS as f32).fract();
                push(
                    Solid::Sphere,
                    point_along(path, share * length),
                    Vec3::splat(BEAD_MM * 2.0),
                    Mat3::IDENTITY,
                    bright,
                    self.lit(Owner::Edge(edge), focus),
                );
            }
        }
    }
}

/// What the ray meets of a volume: a node or an edge, by its index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Met {
    Node(usize),
    Edge(usize),
}

/// How large a volume's labels are drawn with `model`, as the volume is: smaller with it, to [`LEAST_LABEL`] of their
/// own size.
fn label_scale(model: Mat4) -> f32 {
    let (scale, ..) = model.to_scale_rotation_translation();
    // Metres of the scene for each metre of the diagram.
    let drawn = scale.x * 1000.0;
    drawn.clamp(LEAST_LABEL, 1.0) / (PIXEL / LABEL_PIXEL)
}

/// A label's canvas: `text`, bold in `colour`, and below it, for a card or a focus, `detail` on up to three rows, on a
/// backing of `opacity`.
fn label(text: &str, detail: Option<&str>, colour: [f32; 4], opacity: f32) -> Canvas {
    let columns = text
        .chars()
        .count()
        .clamp(1, tomori_protocol::diagram::LABEL_CHARS + 1);
    let detail_rows = detail.map_or(Vec::new(), |detail| {
        let mut rows = wrap(
            &[Span::new(detail, TextStyle::plain(DIM))],
            28,
            Wrap::default(),
        );
        rows.truncate(3);
        rows
    });
    let columns = detail_rows
        .iter()
        .map(Vec::len)
        .chain([columns])
        .max()
        .unwrap_or(1);
    let mut canvas = Canvas::new(columns, PIXEL);
    canvas.padding = (3, 2);
    canvas.background = [0.02, 0.035, 0.06, opacity];
    canvas.rows = std::iter::once(Row::text(text, TextStyle::bold(colour), columns))
        .chain(detail_rows.into_iter().map(Row::new))
        .collect();
    canvas
}

/// The tubes of an edge along `path`, of `radius`, drawn in `style`: one a segment, or its dashes or dots.
fn tubes(
    path: &[Vec3],
    style: Style,
    radius: f32,
    colour: [f32; 3],
    owner: Owner,
    parts: &mut Vec<Part>,
) {
    let tube = |from: Vec3, to: Vec3, parts: &mut Vec<Part>| {
        let length = (to - from).length();
        parts.push(Part {
            solid: Solid::Tube,
            position: (from + to) / 2.0,
            scale: Vec3::new(radius * 2.0, length, radius * 2.0),
            basis: along(to - from),
            colour,
            owner,
        });
    };
    let length = path_length(path);
    match style {
        Style::Dashed => {
            let step = (DASH_MM.0 + DASH_MM.1).max(length / DASHES as f32);
            let dash = step * DASH_MM.0 / (DASH_MM.0 + DASH_MM.1);
            let mut at = 0.0;
            while at < length {
                tube(
                    point_along(path, at),
                    point_along(path, (at + dash).min(length)),
                    parts,
                );
                at += step;
            }
        }
        Style::Dotted => {
            let step = DOT_MM.max(length / DASHES as f32);
            let mut at = step / 2.0;
            while at < length {
                parts.push(Part {
                    solid: Solid::Sphere,
                    position: point_along(path, at),
                    scale: Vec3::splat(radius * 3.0),
                    basis: Mat3::IDENTITY,
                    colour,
                    owner,
                });
                at += step;
            }
        }
        _ => {
            for pair in path.windows(2) {
                tube(pair[0], pair[1], parts);
            }
        }
    }
}

fn path_length(path: &[Vec3]) -> f32 {
    path.windows(2)
        .map(|pair| (pair[1] - pair[0]).length())
        .sum()
}

/// The point `distance` along `path`, held to its ends.
fn point_along(path: &[Vec3], distance: f32) -> Vec3 {
    let mut left = distance.max(0.0);
    for pair in path.windows(2) {
        let length = (pair[1] - pair[0]).length();
        if left <= length {
            return pair[0].lerp(pair[1], if length > 0.0 { left / length } else { 0.0 });
        }
        left -= length;
    }
    path[path.len() - 1]
}

/// The middle of `path`, halfway along it.
fn middle_of(path: &[Vec3]) -> Vec3 {
    point_along(path, path_length(path) / 2.0)
}

/// How far along the ray from `origin` towards `direction`, in its own units, it passes nearest the segment from `a` to
/// `b`, and how far from it it passes there.
fn passing(origin: Vec3, direction: Vec3, a: Vec3, b: Vec3) -> (f32, f32) {
    let segment = b - a;
    let across = origin - a;
    let (dd, ss, ds) = (
        direction.dot(direction),
        segment.dot(segment),
        direction.dot(segment),
    );
    let (da, sa) = (direction.dot(across), segment.dot(across));
    let denominator = dd * ss - ds * ds;
    // Parallel, the segment's start stands for it.
    let mut along_segment = if denominator.abs() > 1e-9 {
        ((dd * sa - ds * da) / denominator).clamp(0.0, 1.0)
    } else {
        0.0
    };
    if ss <= 1e-9 {
        along_segment = 0.0;
    }
    let point = a + segment * along_segment;
    let along_ray = ((point - origin).dot(direction) / dd).max(0.0);
    (along_ray, (origin + direction * along_ray - point).length())
}

/// How many columns and rows of body a volume's face has, sized as `fit` says: a slot's, or, floating, as large as its
/// diagram, its longest side within [`FLOATING`], and never narrower than its title.
pub(super) fn fit(panel: &Panel, diagram: &Diagram, fit: Fit) -> (usize, usize) {
    match fit {
        Fit::Slot(cells) => (cells.columns, cells.body),
        Fit::Text => {
            let (width, height) = floating_size(diagram);
            let columns = (width / (CELL_WIDTH as f32 * PIXEL)).ceil() as usize;
            let rows = (height / (CELL_HEIGHT as f32 * PIXEL)).ceil() as usize;
            (
                columns
                    .max(super::heading_width(panel))
                    .clamp(super::FIT_COLUMNS.0, TABLE_COLUMNS),
                rows.clamp(FIT_ROWS.0, FIT_ROWS.1),
            )
        }
    }
}

/// How wide and tall a volume of `diagram` floats, in metres, at its own scale: its longest side within [`FLOATING`].
fn floating_size(diagram: &Diagram) -> (f32, f32) {
    let extent = diagram.extent.map(|side| f32::from(side.max(1)) / 1000.0);
    let longest = extent.iter().copied().fold(0.0, f32::max);
    let scale = longest.clamp(FLOATING.0, FLOATING.1) / longest;
    (extent[0] * scale, extent[1] * scale)
}

/// A volume's face, sized as `fit` says, framed by `border`: its title, bold, its metadata and age, a rule, and below
/// them the plate its diagram stands in front of, dark in a slot and faint floating.
pub(super) fn face(panel: &Panel, diagram: &Diagram, fit: Fit, border: Option<[f32; 4]>) -> Canvas {
    let (columns, body) = self::fit(panel, diagram, fit);
    let mut canvas = Canvas::new(columns, PIXEL);
    canvas.padding = PADDING;
    canvas.background = match fit {
        Fit::Slot(_) => [0.015, 0.025, 0.045, 0.94],
        Fit::Text => [0.02, 0.035, 0.06, 0.3],
    };
    canvas.border = border;
    canvas.rows = vec![
        Row::text(&panel.title, TextStyle::bold(BRIGHT), columns),
        Row::text(&super::about(panel), TextStyle::plain(DIM), columns),
        Mark::Rule.row(columns, TextStyle::plain(DIM)),
    ];
    canvas
        .rows
        .extend(std::iter::repeat_n(Row::new(Vec::new()), body));
    canvas
}

/// Where a volume of `diagram` stands on its face, sized as `fit` says: from its own millimetres into the face's own
/// metres, its middle in the middle of the plate below its heading, as large as fits there, no deeper than half its
/// height in a slot, and its back just in front of the face.
pub fn placed(panel: &Panel, diagram: &Diagram, fit: Fit) -> Mat4 {
    let (columns, body) = self::fit(panel, diagram, fit);
    let (width, height) = super::sized(columns, body);
    let area = (
        (columns * CELL_WIDTH) as f32 * PIXEL,
        (body * CELL_HEIGHT) as f32 * PIXEL,
    );
    let top = height / 2.0 - (PADDING.1 + 3 * CELL_HEIGHT) as f32 * PIXEL;
    let left = -width / 2.0 + PADDING.0 as f32 * PIXEL;
    let middle = Vec3::new(left + area.0 / 2.0, top - area.1 / 2.0, 0.0);
    let extent = diagram.extent.map(|side| f32::from(side.max(1)));
    let mut scale = (area.0 / extent[0]).min(area.1 / extent[1]);
    if matches!(fit, Fit::Slot(_)) {
        scale = scale.min(area.1 / 2.0 / extent[2]);
    }
    let depth = extent[2] * scale;
    Mat4::from_translation(middle + Vec3::Z * (depth / 2.0 + STAND_OFF))
        * Mat4::from_scale(Vec3::splat(scale))
}

/// A volume as a frame draws it: what it draws, kept across frames, from its own millimetres into the scene, the node
/// the participant focuses on, and how far into the flows' movement it is. See `design/diagrams.md`.
#[derive(Clone, Debug)]
pub struct Drawn {
    pub mesh: Arc<Mesh>,
    pub model: Mat4,
    pub focus: Option<usize>,
    pub time: f32,
}

impl PartialEq for Drawn {
    /// Two are alike where they draw the same mesh alike.
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.mesh, &other.mesh)
            && self.model == other.model
            && self.focus == other.focus
            && self.time == other.time
    }
}

impl Drawn {
    /// Its solids in the scene.
    pub fn instances(&self, out: &mut Vec<Instance>) {
        self.mesh.instances(self.model, self.focus, self.time, out);
    }
}

#[cfg(test)]
mod tests;
