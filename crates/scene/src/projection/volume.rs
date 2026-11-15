// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Volumes: panels with depth, each showing the diagram the server laid out, its nodes as lit solids in their groups'
//! colours and their kinds' shapes, its edges as tubes curved through their waypoints from surface to surface, narrowing
//! towards where they point, its layers as floors and containers as frames, a legend of its groups and kinds on its
//! face, and its labels from the shared glyph atlas, each turned to face the viewer about the vertical and brought in
//! front of its solids. Its face is a panel's, its title above a plate the diagram stands in front of: a dark one in a
//! wall's slot, as a shallow display case, and a faint one floating, so that what is behind it shows between its nodes.
//! What it draws is built once for each version of its diagram and kept, as a panel's face is; a frame moves it by its
//! panel's model alone, and dims what is not the node the participant focuses on and its neighbours. The ray meets its
//! nodes and edges, nearest first, which name them to the agent. See `design/diagrams.md`.
use std::sync::Arc;

use glam::{Mat3, Mat4, Vec3};
use tomori_protocol::Panel;
use tomori_protocol::diagram::{
    Arrow, Diagram, GROUP_COLOURS, GroupKind, LABEL_LINES, Shape, Style,
};

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

/// The widest a label's own text is drawn, in columns: a longer line is wrapped at its words onto the next, and of at
/// most [`LABEL_LINES`] rows the last is cut with "…" where more would follow.
const LABEL_COLUMNS: usize = 20;

/// The nearest a label brought in front of its volume comes to the viewer, in metres, unless it stood nearer.
const NEAREST: f32 = 0.4;

/// How far away a volume floating and one in a wall's slot are read from, in metres, from which its front is seen
/// within its plate.
const FLOATING_READ: f32 = 1.3;
const SLOT_READ: f32 = 2.0;
/// How far a volume's back stands in front of its face, in metres, so that its plate is behind everything it draws.
const STAND_OFF: f32 = 0.01;

/// The radius of an edge's tube where it leaves its source and where it reaches its target, narrowing between so that
/// which way it points reads from any side; of one with no direction, or both; and of the beads moving along a flow
/// and along the edges of the node the participant focuses on, in millimetres of the volume.
const TAPER_MM: (f32, f32) = (3.5, 1.4);
const TUBE_MM: f32 = 2.4;
const BEAD_MM: f32 = 5.5;
/// How much narrower a [`Solid::Taper`] is at its top than at its base, as [`TAPER_MM`] narrows: a straight edge that
/// points is one, of as few triangles as a tube.
pub const TAPER_RATIO: f32 = TAPER_MM.1 / TAPER_MM.0;
/// How many tubes a curved edge that points is drawn in, along it, each a little narrower than the last.
const TAPER_PIECES: usize = 8;
/// An arrowhead's radius and length where an edge points, in millimetres of the volume, and how far along the edge its
/// middle one's point stands, as a share.
const ARROW_MM: (f32, f32) = (14.0, 34.0);
const MIDDLE_ARROW: f32 = 0.6;
/// How far towards white an arrowhead is from its edge's colour.
const ARROW_LIGHT: f32 = 0.6;
/// How many pieces each span of an edge's curve through its waypoints is drawn in.
const CURVE_PIECES: usize = 8;
/// How far above a node's surface its label stands, and above an edge's middle its own, in millimetres.
const NODE_LABEL_MM: f32 = 18.0;
const EDGE_LABEL_MM: f32 = 12.0;
/// How far a group's box reaches beyond its members' as they are pulled about, in millimetres.
const GROUP_MARGIN_MM: f32 = 20.0;
/// How far within a node's surface an edge ends, in millimetres, so that it meets it without a seam.
const EMBED_MM: f32 = 1.0;
/// A dashed edge's dashes and gaps, and a dotted one's spacing, in millimetres, and the most of either along an edge.
const DASH_MM: (f32, f32) = (24.0, 16.0);
const DOT_MM: f32 = 22.0;
const DASHES: usize = 24;
/// How many beads move along a flow, and how fast, in millimetres a second.
const BEADS: usize = 3;
const FLOW_MM: f32 = 120.0;

/// How bright what the participant does not focus on is drawn, as a share.
const DIMMED: f32 = 0.2;
/// An edge's colour between nodes of different groups, unless it has one, and a group's floor's or frame's without a
/// colour of its own.
const EDGE: [f32; 3] = [0.42, 0.46, 0.53];
const NEUTRAL_GROUP: [f32; 3] = [0.34, 0.4, 0.5];
/// How bright an edge within a group is drawn in the group's colour, and a layer's floor's and a container's frame's
/// bars, as shares.
const WITHIN: f32 = 0.75;
const FRAMED: f32 = 0.7;
/// How thick a layer's floor's and a container's frame's bars are, in millimetres.
const BAR_MM: f32 = 2.5;
/// How far towards white a nested group's colour is from its parent's, for each level.
const NESTED: f32 = 0.3;

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
    /// A thin tube narrowing along its height, to [`TAPER_RATIO`] at its top, as a straight edge that points.
    Taper,
}

impl Solid {
    /// Every solid, in the order the renderer keeps their meshes.
    pub const ALL: [Solid; 8] = [
        Solid::Box,
        Solid::Sphere,
        Solid::Cylinder,
        Solid::Prism,
        Solid::Diamond,
        Solid::Cone,
        Solid::Tube,
        Solid::Taper,
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
    Group(usize),
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
#[derive(Clone, Debug)]
pub struct Mesh {
    /// The diagram as it was sent, by which it is kept, and as it is drawn, within its bounds.
    source: Diagram,
    diagram: Diagram,
    parts: Vec<Part>,
    labels: Vec<Label>,
    /// Each node's middle and radius.
    nodes: Vec<(Vec3, f32)>,
    /// Each edge's path, from surface to surface, from where it points from to where it points, and its colour.
    edges: Vec<(Vec<Vec3>, [f32; 3])>,
    /// The edges that point one way, whose beads move while the participant focuses on either end.
    pointing: Vec<bool>,
    /// The edges that flow.
    flows: Vec<usize>,
    /// Each node's detail card, for a node with a detail, which the participant focusing on it sees beside it.
    cards: Vec<Option<Arc<Canvas>>>,
    /// Its legend, of its groups and kinds, kept with it.
    legend: Vec<Entry>,
    /// Its world edges to things here, drawn in the space.
    leaders: Vec<leaders::Leader>,
    /// The nodes and edges lit, the rest dimmed, while no node is focused on, as a path between two held.
    highlight: Option<Highlight>,
}

/// Which nodes and edges of a volume are lit, by their indices, the rest dimmed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Highlight {
    pub nodes: Vec<usize>,
    pub edges: Vec<usize>,
}

/// A node's grey, the colour a node in no group is unless it says another, and an explicit grey: as bright as the chart
/// palette's grey was before it was darkened to tell a chart's eighth series apart, since a lit solid in the space is
/// not text on a dark face.
const NODE_GREY: [f32; 3] = [0.56, 0.6, 0.66];

/// A node's or an edge's colour, as the palette has it, but for grey, [`NODE_GREY`].
fn rgb(colour: tomori_protocol::chart::Colour) -> [f32; 3] {
    match colour {
        tomori_protocol::chart::Colour::Grey => NODE_GREY,
        colour => colour.rgb(),
    }
}

/// Each group's colour: the palette's, told apart with colour blindness (`Colour::SERIES`), for each group at the top
/// in the order written, its own nested groups' paler for each level, and none past [`GROUP_COLOURS`] at the top.
pub fn group_colours(diagram: &Diagram) -> Vec<Option<[f32; 3]>> {
    let mut colours: Vec<Option<[f32; 3]>> = Vec::new();
    let mut tops = 0;
    for group in &diagram.groups {
        let colour = match group
            .parent
            .and_then(|parent| colours.get(usize::from(parent)))
        {
            Some(parent) => {
                parent.map(|colour| colour.map(|channel| channel + (1.0 - channel) * NESTED))
            }
            None => {
                tops += 1;
                tomori_protocol::chart::Colour::SERIES
                    .get(tops - 1)
                    .filter(|_| tops <= GROUP_COLOURS)
                    .map(|colour| colour.rgb())
            }
        };
        colours.push(colour);
    }
    colours
}

/// The colour of `node` of `diagram`, whose groups are coloured `groups`: its own, or else its group's, or else grey.
fn node_colour(node: &tomori_protocol::diagram::Node, groups: &[Option<[f32; 3]>]) -> [f32; 3] {
    match node.colour {
        tomori_protocol::chart::Colour::Grey => node
            .group
            .and_then(|group| groups.get(usize::from(group)).copied().flatten())
            .unwrap_or(NODE_GREY),
        colour => colour.rgb(),
    }
}

/// How far from a node's middle its solid's surface is along `direction`, a unit vector, in its millimetres: `solid`
/// scaled by `scale`, as it is drawn, faceted as its mesh is.
pub fn surface(solid: Solid, scale: Vec3, direction: Vec3) -> f32 {
    let half = scale / 2.0;
    let d = direction.abs();
    let reach = |extent: f32, part: f32| {
        if part > 1e-6 {
            extent / part
        } else {
            f32::INFINITY
        }
    };
    match solid {
        // Its faces between its corners, a little within the sphere about them.
        Solid::Sphere => half.x * 0.92,
        Solid::Diamond => 1.0 / (d.x / half.x + d.y / half.y + d.z / half.z),
        Solid::Cylinder | Solid::Prism => {
            let sides = if solid == Solid::Cylinder { 12.0 } else { 6.0 };
            let sector = std::f32::consts::TAU / sides;
            let across = direction.x.hypot(direction.z);
            let side = if across > 1e-6 {
                let angle = direction.z.atan2(direction.x).rem_euclid(sector);
                reach(
                    half.x * (sector / 2.0).cos() / (angle - sector / 2.0).cos(),
                    across,
                )
            } else {
                f32::INFINITY
            };
            side.min(reach(half.y, d.y))
        }
        _ => reach(half.x, d.x)
            .min(reach(half.y, d.y))
            .min(reach(half.z, d.z)),
    }
}

/// The solid a node of `shape`, `width` across, is drawn as, its size along each of its axes, and which way they point:
/// a prism lies with a hexagon, pointed to either side, towards the front, so that it is not taken for a box seen
/// from there.
fn body(shape: Shape, width: f32) -> (Solid, Vec3, Mat3) {
    let upright = Mat3::IDENTITY;
    match shape {
        Shape::Sphere => (Solid::Sphere, Vec3::splat(width), upright),
        Shape::Cylinder => (
            Solid::Cylinder,
            Vec3::new(width, width * 0.8, width),
            upright,
        ),
        Shape::Prism => (
            Solid::Prism,
            Vec3::new(width, width * 0.5, width),
            Mat3::from_rotation_x(std::f32::consts::FRAC_PI_2),
        ),
        Shape::Diamond => (Solid::Diamond, Vec3::splat(width), upright),
        Shape::Card => (
            Solid::Box,
            Vec3::new(width * 1.4, width, width * 0.12),
            upright,
        ),
        _ => (
            Solid::Box,
            Vec3::new(width, width * 0.7, width * 0.7),
            upright,
        ),
    }
}

/// `points` curved smoothly through each of them, a Catmull–Rom spline of [`CURVE_PIECES`] pieces a span; two points,
/// a straight line, as they are.
fn curved(points: &[Vec3]) -> Vec<Vec3> {
    if points.len() <= 2 {
        return points.to_vec();
    }
    let last = points.len() - 1;
    let mut curve = vec![points[0]];
    for span in 0..last {
        let (p1, p2) = (points[span], points[span + 1]);
        let p0 = if span == 0 {
            p1 * 2.0 - p2
        } else {
            points[span - 1]
        };
        let p3 = if span + 1 == last {
            p2 * 2.0 - p1
        } else {
            points[span + 2]
        };
        for piece in 1..=CURVE_PIECES {
            let t = piece as f32 / CURVE_PIECES as f32;
            let (t2, t3) = (t * t, t * t * t);
            curve.push(
                0.5 * (p1 * 2.0
                    + (p2 - p0) * t
                    + (p0 * 2.0 - p1 * 5.0 + p2 * 4.0 - p3) * t2
                    + (p1 * 3.0 - p0 - p2 * 3.0 + p3) * t3),
            );
        }
    }
    curve
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
        Self::built(source, source.bounded(), None)
    }

    /// It as drawn while a participant pulls its nodes about, its nodes where the server laid them out, its edges
    /// straight between them and its groups by their colour alone, its labels' canvases kept; [`Mesh::repose`] then
    /// moves its nodes. See [`springs`].
    pub fn posed(&self) -> Self {
        let mut diagram = self.diagram.clone();
        for edge in &mut diagram.edges {
            edge.via.clear();
        }
        // Drawn by its colour alone while its members move, a floor or frame stretching after them being only more to
        // follow.
        for group in &mut diagram.groups {
            group.kind = GroupKind::Cluster;
        }
        Self::built(&self.source, diagram, Some(self))
    }

    /// Move its nodes to `at`, in its millimetres, as a participant pulls them about, where they stand to a fraction of a
    /// millimetre: its nodes' solids, its edges' solids along their new paths, its labels with what they name and its
    /// extent the box about its nodes, before which its labels are brought; and light, where `highlight` says, those
    /// nodes and edges and dim the rest. Nothing else is made again: its labels' canvases, its legend and its cards are
    /// kept. A mesh from [`Mesh::posed`], whose edges have no waypoints and whose groups are drawn as nothing but colour.
    pub fn repose(&mut self, at: &[Vec3], highlight: Option<Highlight>) {
        let count = self.nodes.len().min(at.len());
        for (index, at) in at.iter().enumerate().take(count) {
            self.nodes[index].0 = *at;
            self.parts[index].position = *at;
        }
        self.parts.truncate(self.diagram.nodes.len());
        for (index, edge) in self.diagram.edges.iter().enumerate() {
            let (from, to) = (usize::from(edge.from), usize::from(edge.to));
            let body = |node: usize| {
                let node = &self.diagram.nodes[node];
                body(node.shape, node.width())
            };
            let path = edge_path(
                edge,
                (self.nodes[from].0, self.nodes[to].0),
                &[],
                (body(from), body(to)),
            );
            edge_parts(edge, index, &path, self.edges[index].1, &mut self.parts);
            self.edges[index].0 = path;
        }
        // Each group's label about its members as they stand.
        let mut bounds: Vec<Option<(Vec3, Vec3)>> = vec![None; self.diagram.groups.len()];
        for (node, (at, radius)) in self.diagram.nodes.iter().zip(&self.nodes) {
            let mut group = node.group;
            while let Some(index) = group.map(usize::from).filter(|index| *index < bounds.len()) {
                let reach = Vec3::splat(radius + GROUP_MARGIN_MM);
                let (low, high) = bounds[index].unwrap_or((*at - reach, *at + reach));
                bounds[index] = Some((low.min(*at - reach), high.max(*at + reach)));
                group = self.diagram.groups[index].parent;
            }
        }
        for label in &mut self.labels {
            label.at = match label.owner {
                Owner::Node(node) => {
                    self.nodes[node].0 + Vec3::Y * (self.nodes[node].1 + NODE_LABEL_MM)
                }
                Owner::Edge(edge) => middle_of(&self.edges[edge].0) + Vec3::Y * EDGE_LABEL_MM,
                Owner::Group(group) => match bounds[group] {
                    Some((low, high)) => Vec3::new(low.x, high.y, high.z),
                    None => label.at,
                },
            };
        }
        let reach = self.nodes.iter().fold(Vec3::ZERO, |reach, (at, radius)| {
            reach.max(at.abs() + Vec3::splat(radius + GROUP_MARGIN_MM))
        });
        let most = f32::from(tomori_protocol::diagram::EXTENT_MM);
        self.diagram.extent = (reach * 2.0)
            .to_array()
            .map(|side| side.round().clamp(1.0, most) as u16);
        self.highlight = highlight;
    }

    /// What `diagram`, as `source` holds it within its bounds, draws, its labels' and cards' canvases those of `kept`,
    /// of the same diagram, where it is given.
    fn built(source: &Diagram, diagram: Diagram, kept: Option<&Mesh>) -> Self {
        let mut canvases = kept.map(|mesh| mesh.labels.iter().map(|label| label.canvas.clone()));
        let mut canvas = |make: &dyn Fn() -> Canvas| {
            canvases
                .as_mut()
                .and_then(Iterator::next)
                .unwrap_or_else(|| Arc::new(make()))
        };
        let point = |at: [i16; 3]| Vec3::new(f32::from(at[0]), f32::from(at[1]), f32::from(at[2]));
        let mut parts = Vec::new();
        let mut labels = Vec::new();
        let nodes: Vec<(Vec3, f32)> = diagram
            .nodes
            .iter()
            .map(|node| (point(node.at), node.width() / 2.0))
            .collect();
        let groups = group_colours(&diagram);
        let bodies: Vec<(Solid, Vec3, Mat3)> = diagram
            .nodes
            .iter()
            .map(|node| body(node.shape, node.width()))
            .collect();
        for (index, node) in diagram.nodes.iter().enumerate() {
            let (solid, scale, basis) = bodies[index];
            parts.push(Part {
                solid,
                position: nodes[index].0,
                scale,
                basis,
                colour: node_colour(node, &groups),
                owner: Owner::Node(index),
            });
        }
        let mut edges = Vec::new();
        let mut flows = Vec::new();
        let mut pointing = Vec::new();
        for (index, edge) in diagram.edges.iter().enumerate() {
            let (from, to) = (usize::from(edge.from), usize::from(edge.to));
            let via: Vec<Vec3> = edge.via.iter().copied().map(point).collect();
            let path = edge_path(
                edge,
                (nodes[from].0, nodes[to].0),
                &via,
                (bodies[from], bodies[to]),
            );
            let shared = diagram.nodes[from]
                .group
                .filter(|group| Some(*group) == diagram.nodes[to].group)
                .and_then(|group| groups.get(usize::from(group)).copied().flatten());
            let colour = edge.colour.map_or_else(
                || shared.map_or(EDGE, |colour| colour.map(|channel| channel * WITHIN)),
                rgb,
            );
            let points = edge_parts(edge, index, &path, colour, &mut parts);
            if edge.style == Style::Flow {
                flows.push(index);
            }
            pointing.push(points);
            edges.push((path, colour));
        }
        for (index, group) in diagram.groups.iter().enumerate() {
            let (low, high) = (point(group.low), point(group.high));
            let size = (high - low).max(Vec3::ONE);
            let middle = (low + high) / 2.0;
            let colour = groups[index];
            match group.kind {
                // A cluster is told by its members' colour alone.
                GroupKind::Cluster => {}
                GroupKind::Container => {
                    // Its twelve edges, as a frame.
                    let colour = colour.map_or(NEUTRAL_GROUP, |colour| {
                        colour.map(|channel| channel * FRAMED)
                    });
                    for axis in 0..3 {
                        let mut scale = Vec3::splat(BAR_MM);
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
                                colour,
                                owner: Owner::Group(index),
                            });
                        }
                    }
                }
                // A layer's floor beneath its members, its four edges, thin, in its colour, which hide nothing behind it.
                GroupKind::Layer => {
                    let colour = colour.map_or(NEUTRAL_GROUP, |colour| {
                        colour.map(|channel| channel * FRAMED)
                    });
                    for (axis, across) in [(0, 2), (2, 0)] {
                        let mut scale = Vec3::splat(BAR_MM);
                        scale[axis] = size[axis];
                        for side in [low[across], high[across]] {
                            let mut position = Vec3::new(middle.x, low.y, middle.z);
                            position[across] = side;
                            parts.push(Part {
                                solid: Solid::Box,
                                position,
                                scale,
                                basis: Mat3::IDENTITY,
                                colour,
                                owner: Owner::Group(index),
                            });
                        }
                    }
                }
            }
            // A group with a colour is named by the legend; one without, past the palette, by a label of its own.
            if colour.is_none() {
                labels.push(Label {
                    canvas: canvas(&|| label(group_name(group), None, DIM, 0.6)),
                    at: Vec3::new(low.x, high.y, high.z),
                    owner: Owner::Group(index),
                });
            }
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
                canvas: canvas(&|| label(&node.label, detail, BRIGHT, 0.85)),
                at: at + Vec3::Y * (radius + NODE_LABEL_MM),
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
                canvas: canvas(&|| label(&edge.label, None, DIM, 0.7)),
                at: middle_of(path) + Vec3::Y * EDGE_LABEL_MM,
                owner: Owner::Edge(index),
            });
        }
        let cards = match kept {
            Some(mesh) => mesh.cards.clone(),
            None => diagram
                .nodes
                .iter()
                .map(|node| {
                    (!node.detail.is_empty())
                        .then(|| Arc::new(label(&node.label, Some(&node.detail), BRIGHT, 0.92)))
                })
                .collect(),
        };
        Self {
            cards,
            legend: legend(&diagram),
            highlight: None,
            leaders: leaders::Leader::of(&diagram),
            source: source.clone(),
            diagram,
            parts,
            labels,
            nodes,
            edges,
            pointing,
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

    /// How many solids it draws, flows' beads besides, but its leaders, which are drawn afresh each frame.
    pub fn solids(&self) -> usize {
        self.parts.len() + self.flows.len() * BEADS
    }

    /// How many leaders it draws, a world edge each to a thing here.
    pub fn leaders(&self) -> usize {
        self.leaders.len()
    }

    /// The world edge its leader `index` draws, by its index among its diagram's world edges, which a u16 holds, as at
    /// most [`tomori_protocol::diagram::WORLD_EDGES`] are drawn.
    pub fn world_edge(&self, index: usize) -> u16 {
        self.leaders
            .get(index)
            .map_or(0, |leader| leader.edge as u16)
    }

    /// The node its leader `index` comes from, whose focus that leader's focus is.
    pub fn leader_node(&self, index: usize) -> Option<usize> {
        self.leaders.get(index).map(|leader| leader.node)
    }

    /// Each leader's path in the space, drawn with `model`, from its node's surface to its mark.
    fn leader_paths(&self, model: Mat4) -> Vec<Vec<Vec3>> {
        let (scale, ..) = model.to_scale_rotation_translation();
        self.leaders
            .iter()
            .map(|leader| {
                let (at, radius) = self.nodes[leader.node];
                let out = leaders::out_of(model, self.diagram.extent, leader.target());
                leader.path(model.transform_point3(at), radius * scale.x, out)
            })
            .collect()
    }

    /// Where the node at `index` stands, in its millimetres, or its middle for one it does not have.
    pub fn node_at(&self, index: usize) -> Vec3 {
        self.nodes.get(index).map_or(Vec3::ZERO, |(at, _)| *at)
    }

    /// How many labels it draws.
    pub fn labels(&self) -> usize {
        self.labels.len()
    }

    /// The node, edge or leader `ray` meets first, in the volume drawn with `model`, from its own millimetres into the
    /// scene, and how far along the ray: a node by its bounds, an edge within [`EDGE_REACH`] of its path, and a leader
    /// within it of its line or its mark, in the space.
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
        for (index, (leader, path)) in self
            .leaders
            .iter()
            .zip(self.leader_paths(model))
            .enumerate()
        {
            let mark = leader.mark_path().unwrap_or_default();
            for found in [leaders::met(&path, ray), leaders::met(&mark, ray)]
                .into_iter()
                .flatten()
            {
                take(Met::Leader(index), found);
            }
        }
        best
    }

    /// The labels it draws with `model`, each turned about the vertical to face `viewer`, sized as the volume is drawn
    /// and brought in front of everything it draws ([`Self::before`]), farthest first, what the participant does not
    /// focus on, `focus`, left out.
    pub fn sheets(&self, model: Mat4, viewer: Vec3, focus: Option<usize>, sheets: &mut Vec<Sheet>) {
        // A leader's label stands at its target, while it is drawn in full.
        for (index, (leader, path)) in self
            .leaders
            .iter()
            .zip(self.leader_paths(model))
            .enumerate()
        {
            if leaders::strength(&self.leaders, index, focus).0 >= 1.0 {
                sheets.push(leader.sheet(leader.label_at(path[0]), viewer));
            }
        }
        let scale = label_scale(model);
        let mut drawn: Vec<(f32, Sheet)> = Vec::new();
        for label in &self.labels {
            if !self.lit(label.owner, focus) {
                continue;
            }
            // Its first row where a label of one stands, the rest growing upwards, away from what it names.
            let rows = label.canvas.rows.len().saturating_sub(1);
            let lift = (rows * CELL_HEIGHT) as f32 * PIXEL / 2.0 * scale;
            let at = model.transform_point3(label.at) + Vec3::Y * lift;
            let placed = crate::recording::facing(at, Vec3::new(viewer.x, at.y, viewer.z), scale);
            let placed = self.before(model, viewer, placed, &label.canvas);
            drawn.push((
                at.distance(viewer),
                Sheet {
                    canvas: Some(label.canvas.clone()),
                    model: Some(placed),
                    ahead: false,
                },
            ));
        }
        // Farthest first by where they stood, before they were brought forward, so that where two overlap the label of
        // what is nearer is drawn over the label of what is behind it.
        drawn.sort_by(|a, b| b.0.total_cmp(&a.0));
        sheets.extend(drawn.into_iter().map(|(_, sheet)| sheet));
    }

    /// `placed`, the model of a label of `canvas`, brought in front of everything the volume drawn with `model` draws,
    /// as `viewer` sees it, so that no node, edge or group of it hides the label: moved towards the viewer to where the
    /// line from them to its middle enters the box of the volume's extent, grown by the label's half diagonal, and
    /// shrunk about the viewer as it comes, so that it is seen exactly as large and in the same place. It never comes
    /// nearer the viewer than [`NEAREST`], unless it stood nearer, so that leaning in to read a diagram, or seen from
    /// within its box, where it enters at the eyes, its labels stand no nearer than that, and move smoothly as the head
    /// crosses into the box. A label already in front of that box stays where it is. It never leaves the box so grown,
    /// so that whatever stands in front of the volume, a panel, a wall or a hand, still hides it by depth, but for a
    /// panel within a label's half diagonal of the box. See `design/diagrams.md`.
    fn before(&self, model: Mat4, viewer: Vec3, placed: Mat4, canvas: &Canvas) -> Mat4 {
        let (width, height) = canvas.size();
        let (scale, ..) = model.to_scale_rotation_translation();
        // The label's half diagonal, in the volume's millimetres.
        let reach = placed.x_axis.truncate().length() * width.hypot(height) / 2.0 / scale.x;
        let half = Vec3::from_array(self.diagram.extent.map(f32::from)) / 2.0 + Vec3::splat(reach);
        let inverse = model.inverse();
        let middle = placed.transform_point3(Vec3::ZERO);
        match entering(
            inverse.transform_point3(viewer),
            inverse.transform_point3(middle),
            half,
        ) {
            Some(entry) if entry < 1.0 => {
                let distance = middle.distance(viewer);
                let share = entry.max(NEAREST.min(distance) / distance.max(f32::EPSILON));
                Mat4::from_translation(viewer)
                    * Mat4::from_scale(Vec3::splat(share))
                    * Mat4::from_translation(-viewer)
                    * placed
            }
            _ => placed,
        }
    }

    /// Whether what `owner` stands for is lit while the participant focuses on the node `focus`: it, the nodes it is
    /// joined to and the edges between them; everything, without a focus.
    fn lit(&self, owner: Owner, focus: Option<usize>) -> bool {
        let Some(focus) = focus else {
            return match (&self.highlight, owner) {
                (None, _) => true,
                (Some(lit), Owner::Node(node)) => lit.nodes.contains(&node),
                (Some(lit), Owner::Edge(edge)) => lit.edges.contains(&edge),
                (Some(_), Owner::Group(_)) => false,
            };
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
            Owner::Group(_) => false,
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
        let placed = crate::recording::facing(at, Vec3::new(viewer.x, at.y, viewer.z), scale)
            * Mat4::from_translation(Vec3::X * width / 2.0);
        Some(Sheet {
            model: Some(self.before(model, viewer, placed, &canvas)),
            canvas: Some(canvas),
            ahead: false,
        })
    }

    /// Its solids drawn with `model`, from its own millimetres into the scene, `time` seconds into the flows' movement,
    /// what the participant does not focus on, `focus`, dimmed: each a unit solid moved, turned and scaled; and its
    /// leaders, in the space, as seen from `viewer`.
    pub fn instances(
        &self,
        model: Mat4,
        focus: Option<usize>,
        time: f32,
        viewer: Vec3,
        out: &mut Vec<Instance>,
    ) {
        for (index, (leader, path)) in self
            .leaders
            .iter()
            .zip(self.leader_paths(model))
            .enumerate()
        {
            leader.instances(
                &path,
                viewer,
                time,
                leaders::strength(&self.leaders, index, focus),
                out,
            );
        }
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
        // A flow's beads, and, while the participant focuses on a node, those of the edges that point to or from it.
        let focused = |edge: usize| {
            let edge = &self.diagram.edges[edge];
            focus.is_some_and(|focus| {
                usize::from(edge.from) == focus || usize::from(edge.to) == focus
            })
        };
        let highlighted = |edge: usize| {
            self.highlight
                .as_ref()
                .is_some_and(|lit| lit.edges.contains(&edge))
        };
        let beading = (0..self.edges.len()).filter(|edge| {
            self.flows.contains(edge)
                || (self.pointing[*edge] && (focused(*edge) || highlighted(*edge)))
        });
        for edge in beading {
            let (path, colour) = &self.edges[edge];
            let length = path_length(path);
            let bright = colour.map(|channel| (channel * 1.6 + 0.15).min(1.0));
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

/// What the ray meets of a volume: a node, an edge or a leader, the line of a world edge to a thing in the space, by its
/// index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Met {
    Node(usize),
    Edge(usize),
    Leader(usize),
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
    let style = TextStyle::bold(colour);
    let mut rows = wrap(&[Span::new(text, style)], LABEL_COLUMNS, Wrap::default());
    if rows.len() > LABEL_LINES {
        rows.truncate(LABEL_LINES);
        let last: String = rows[LABEL_LINES - 1]
            .iter()
            .map(|(ch, _)| *ch)
            .filter(|ch| *ch != crate::ui_text::WIDE)
            .collect();
        rows[LABEL_LINES - 1] =
            Row::text(&format!("{}…", last.trim_end()), style, LABEL_COLUMNS).cells;
    }
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
        .chain(&rows)
        .map(Vec::len)
        .max()
        .unwrap_or(0)
        .max(1);
    let mut canvas = Canvas::new(columns, PIXEL);
    canvas.padding = (3, 2);
    canvas.background = [0.02, 0.035, 0.06, opacity];
    canvas.rows = rows.into_iter().chain(detail_rows).map(Row::new).collect();
    canvas
}

/// How far along the segment from `from` to `to`, as a share of it, it enters the box `half` either way about the
/// origin, 0 where it starts within it; `None` where it misses the box.
fn entering(from: Vec3, to: Vec3, half: Vec3) -> Option<f32> {
    // Within the box, it enters where it starts.
    let direction = to - from;
    let (mut near, mut far) = (f32::NEG_INFINITY, f32::INFINITY);
    for axis in 0..3 {
        if direction[axis] == 0.0 {
            if from[axis].abs() > half[axis] {
                return None;
            }
            continue;
        }
        let (a, b) = (
            (-half[axis] - from[axis]) / direction[axis],
            (half[axis] - from[axis]) / direction[axis],
        );
        near = near.max(a.min(b));
        far = far.min(a.max(b));
    }
    (near <= far && far > 0.0).then_some(near.max(0.0))
}

/// The path of `edge` between its ends' middles `ends`, through `via`, from the surface of each end's solid, `bodies`,
/// as it is drawn to the other's, curved through its waypoints, from where it points from to where it points.
fn edge_path(
    edge: &tomori_protocol::diagram::Edge,
    ends: (Vec3, Vec3),
    via: &[Vec3],
    bodies: ((Solid, Vec3, Mat3), (Solid, Vec3, Mat3)),
) -> Vec<Vec3> {
    let mut path: Vec<Vec3> = std::iter::once(ends.0)
        .chain(via.iter().copied())
        .chain(std::iter::once(ends.1))
        .collect();
    let last = path.len() - 1;
    let start = (path[1] - path[0]).try_normalize().unwrap_or(Vec3::X);
    let end = (path[last] - path[last - 1])
        .try_normalize()
        .unwrap_or(Vec3::X);
    let (solid, scale, basis) = bodies.0;
    path[0] += start * (surface(solid, scale, basis.transpose() * start) - EMBED_MM).max(0.0);
    let (solid, scale, basis) = bodies.1;
    path[last] -= end * (surface(solid, scale, basis.transpose() * -end) - EMBED_MM).max(0.0);
    let mut path = curved(&path);
    // From where it points from, so that it narrows towards where it points and its beads move that way.
    if edge.direction == Arrow::Back {
        path.reverse();
    }
    path
}

/// The solids of `edge`, at `index`, along `path`, in `colour`, into `parts`: its tubes, and, where it points one way,
/// its arrowheads where it points and along its middle, lighter than it, so that which way it points reads at a
/// glance; whether it points one way.
fn edge_parts(
    edge: &tomori_protocol::diagram::Edge,
    index: usize,
    path: &[Vec3],
    colour: [f32; 3],
    parts: &mut Vec<Part>,
) -> bool {
    let heavier = (1.0 + 0.5 * f32::from(edge.weight.saturating_sub(1)))
        * if edge.style == Style::Thick { 2.0 } else { 1.0 };
    let points = matches!(edge.direction, Arrow::Forward | Arrow::Back);
    let width = if points {
        (TAPER_MM.0 * heavier, TAPER_MM.1 * heavier)
    } else {
        (TUBE_MM * heavier, TUBE_MM * heavier)
    };
    let owner = Owner::Edge(index);
    tubes(path, edge.style, width, colour, owner, parts);
    if points {
        parts.extend(arrowheads(path, colour, owner));
    }
    points
}

/// The arrowheads of an edge along `path`, from where it points from to where it points, in a light tone of `colour`:
/// one with its point at the path's end, against the target's surface, and one a little past its middle, clear of the
/// labels above its ends, each as long as [`ARROW_MM`] says or a fifth of the edge, whichever is shorter.
fn arrowheads(path: &[Vec3], colour: [f32; 3], owner: Owner) -> Vec<Part> {
    let length = path_length(path);
    let long = ARROW_MM.1.min(length / 5.0);
    let radius = ARROW_MM.0 * long / ARROW_MM.1;
    // A light tone of the edge's colour, two thirds of the way to white, so that it stands out on the edge.
    let colour = colour.map(|channel| channel + (1.0 - channel) * ARROW_LIGHT);
    [length * MIDDLE_ARROW, length]
        .into_iter()
        .filter_map(|tip_at| {
            let tip = point_along(path, tip_at);
            let direction = (tip - point_along(path, tip_at - long)).try_normalize()?;
            Some(Part {
                solid: Solid::Cone,
                position: tip - direction * long / 2.0,
                scale: Vec3::new(radius * 2.0, long, radius * 2.0),
                basis: along(direction),
                colour,
                owner,
            })
        })
        .collect()
}

/// The tubes of an edge along `path`, from `width`'s first radius at its start to its second at its end, drawn in
/// `style`: pieces narrowing along it, or a segment each of its own width, or its dashes or dots, each as wide as the
/// edge is where it stands.
fn tubes(
    path: &[Vec3],
    style: Style,
    width: (f32, f32),
    colour: [f32; 3],
    owner: Owner,
    parts: &mut Vec<Part>,
) {
    let length = path_length(path);
    // How wide it is, as a radius, a share of the way along it, narrowing evenly by a share of itself.
    let radius = |share: f32| width.0 * (width.1 / width.0).powf(share.clamp(0.0, 1.0));
    let tube = |from: Vec3, to: Vec3, radius: f32, parts: &mut Vec<Part>| {
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
    let share = |at: f32| at / length.max(1e-3);
    match style {
        Style::Dashed => {
            let step = (DASH_MM.0 + DASH_MM.1).max(length / DASHES as f32);
            let dash = step * DASH_MM.0 / (DASH_MM.0 + DASH_MM.1);
            let mut at = 0.0;
            while at < length {
                let to = (at + dash).min(length);
                tube(
                    point_along(path, at),
                    point_along(path, to),
                    radius(share((at + to) / 2.0)),
                    parts,
                );
                at += step;
            }
        }
        Style::Dotted => {
            let step = DOT_MM.max(length / DASHES as f32);
            let mut at = step / 2.0;
            // Each dot a small diamond, of a tenth of a sphere's triangles, so that a volume of every edge dotted is
            // within a frame's budget alone.
            while at < length {
                parts.push(Part {
                    solid: Solid::Diamond,
                    position: point_along(path, at),
                    scale: Vec3::splat(radius(share(at)) * 3.0),
                    basis: Mat3::IDENTITY,
                    colour,
                    owner,
                });
                at += step;
            }
        }
        // Narrowing: straight, a [`Solid::Taper`], whose widths are the edge's at its ends; curved, tubes of even length
        // along it, each as wide as it is at its middle.
        _ if width.0 != width.1 && path.len() == 2 => parts.push(Part {
            solid: Solid::Taper,
            position: (path[0] + path[1]) / 2.0,
            scale: Vec3::new(width.0 * 2.0, length, width.0 * 2.0),
            basis: along(path[1] - path[0]),
            colour,
            owner,
        }),
        _ if width.0 != width.1 => {
            let piece = length / TAPER_PIECES as f32;
            for index in 0..TAPER_PIECES {
                tube(
                    point_along(path, piece * index as f32),
                    point_along(path, piece * (index + 1) as f32),
                    radius((index as f32 + 0.5) / TAPER_PIECES as f32),
                    parts,
                );
            }
        }
        _ => {
            for pair in path.windows(2) {
                tube(pair[0], pair[1], width.0, parts);
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

/// The most rows a volume's legend takes on its face, beyond which it counts what it leaves out, and how many rows
/// above it are kept apart from the diagram, whose nearest nodes, below the eyes, are seen lower than the plate. A
/// legend takes at most a quarter of its face's body, fewer rows on a small slot, counting the rest.
pub const LEGEND_ROWS: usize = 4;
const LEGEND_GAP: usize = 2;
/// How many rows below its heading a face keeps clear of its diagram, so that the labels above its highest nodes stand
/// clear of its title.
const LABEL_ROOM: usize = 2;
/// A legend's entries' colour of text, and of the solids showing its kinds.
const LEGEND_TEXT: [f32; 4] = [0.72, 0.76, 0.82, 1.0];
const LEGEND_SOLID: [f32; 3] = [0.62, 0.66, 0.72];

/// An entry of a volume's legend: a group, by its colour, or a kind, by the solid its nodes are drawn as, and its name.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub text: String,
    pub swatch: Swatch,
}

/// What an entry of a legend shows beside its name.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Swatch {
    /// A dot of a group's colour.
    Colour([f32; 3]),
    /// A small solid, as a kind's nodes are drawn, its size along each of its axes, its longest 1, and which way they
    /// point.
    Solid(Solid, Vec3, Mat3),
}

impl Entry {
    /// How many cells it takes on its row: its swatch, a space, and its name.
    fn cells(&self) -> usize {
        cells(&self.text)
    }
}

/// How many cells an entry named `text` takes on its row: its swatch, a space, and its name.
fn cells(text: &str) -> usize {
    2 + text.chars().count()
}

/// The legend of `diagram`: each group with a colour, by its label, in the order written, then each kind its nodes are
/// of, but the module, a box, which a node of no kind is too, in the order first given; empty for a diagram of neither.
pub fn legend(diagram: &Diagram) -> Vec<Entry> {
    let mut entries: Vec<Entry> = diagram
        .groups
        .iter()
        .zip(group_colours(diagram))
        .filter_map(|(group, colour)| {
            Some(Entry {
                text: group_name(group).to_owned(),
                swatch: Swatch::Colour(colour?),
            })
        })
        .collect();
    for node in kinds(diagram) {
        let (solid, scale, basis) = body(node.shape, 1.0);
        entries.push(Entry {
            text: node.kind.clone(),
            swatch: Swatch::Solid(solid, scale / scale.max_element(), basis),
        });
    }
    entries
}

/// A group's name: its label, or else its id.
fn group_name(group: &tomori_protocol::diagram::Group) -> &str {
    if group.label.is_empty() {
        &group.id
    } else {
        &group.label
    }
}

/// The first node of each kind of `diagram` but the module, in the order written.
fn kinds(diagram: &Diagram) -> Vec<&tomori_protocol::diagram::Node> {
    let mut first: Vec<&tomori_protocol::diagram::Node> = Vec::new();
    for node in &diagram.nodes {
        if !node.kind.is_empty()
            && node.kind != tomori_protocol::diagram::KINDS[0].0
            && !first.iter().any(|known| known.kind == node.kind)
        {
            first.push(node);
        }
    }
    first
}

/// How many cells each entry of the legend of `diagram` takes, as [`legend`] would make them, without making them.
fn legend_widths(diagram: &Diagram) -> Vec<usize> {
    diagram
        .groups
        .iter()
        .zip(group_colours(diagram))
        .filter(|(_, colour)| colour.is_some())
        .map(|(group, _)| cells(group_name(group)))
        .chain(kinds(diagram).into_iter().map(|node| cells(&node.kind)))
        .collect()
}

/// Entries `widths` cells wide laid out in rows of `columns` cells, two apart, at most `most` of them, each entry by its
/// index with the column it begins at, and how many were left out, which the last row counts as `+N`.
fn legend_rows(widths: &[usize], columns: usize, most: usize) -> (Vec<Vec<(usize, usize)>>, usize) {
    let mut rows: Vec<Vec<(usize, usize)>> = Vec::new();
    let mut at = columns;
    for (index, width) in widths.iter().enumerate() {
        if at + width > columns {
            if rows.len() == most {
                return (rows, widths.len() - index);
            }
            rows.push(Vec::new());
            at = 0;
        }
        // A row is begun before the first, as `at` begins past the end of none.
        let last = rows.len() - 1;
        rows[last].push((at, index));
        at += width + 2;
    }
    (rows, 0)
}

/// How many rows a floating face of `diagram` gives its diagram.
fn diagram_rows(diagram: &Diagram) -> usize {
    let (_, height) = floating_size(diagram);
    ((height / (CELL_HEIGHT as f32 * PIXEL)).ceil() as usize).clamp(FIT_ROWS.0, FIT_ROWS.1)
}

/// How many columns a face sized as `fit` says has for `panel` and `diagram`, the most rows its legend may take, so
/// that, with the rows kept apart from the diagram, it takes at most a quarter of the face's body, and how many rows
/// its diagram has, which a floating face grows by its legend's.
fn shape(panel: &Panel, diagram: &Diagram, fit: Fit) -> (usize, usize, usize) {
    match fit {
        Fit::Slot(cells) => (
            cells.columns,
            (cells.body / 4).saturating_sub(LEGEND_GAP).min(LEGEND_ROWS),
            cells.body,
        ),
        Fit::Text => {
            let (width, _) = floating_size(diagram);
            let columns = ((width / (CELL_WIDTH as f32 * PIXEL)).ceil() as usize)
                .max(super::heading_width(panel))
                .clamp(super::FIT_COLUMNS.0, TABLE_COLUMNS);
            let rows = diagram_rows(diagram) + LABEL_ROOM;
            (
                columns,
                (rows / 3).saturating_sub(LEGEND_GAP).min(LEGEND_ROWS),
                rows,
            )
        }
    }
}

/// How many rows the legend of entries `widths` cells wide takes on a face of `columns`, at most `most`, with
/// [`LEGEND_GAP`] above it apart from the diagram; none for none.
fn legend_height(widths: &[usize], columns: usize, most: usize) -> usize {
    match legend_rows(widths, columns.saturating_sub(4), most).0.len() {
        0 => 0,
        rows => rows + LEGEND_GAP,
    }
}

/// The rows of the legend of `entries` on a face of `columns`, at most `most`, each entry a dot of its group's colour,
/// or room for the small solid [`legend_icons`] draws there, and its name.
fn legend_lines(entries: &[Entry], columns: usize, most: usize) -> Vec<Row> {
    let widths: Vec<usize> = entries.iter().map(Entry::cells).collect();
    let (rows, more) = legend_rows(&widths, columns.saturating_sub(4), most);
    let count = rows.len();
    rows.into_iter()
        .enumerate()
        .map(|(index, row)| {
            let mut spans = Vec::new();
            let mut at = 0;
            for (column, entry) in row {
                let entry = &entries[entry];
                spans.push(Span::new(
                    " ".repeat(column - at),
                    TextStyle::plain(LEGEND_TEXT),
                ));
                match entry.swatch {
                    Swatch::Colour(colour) => spans.push(
                        Mark::Dot.span(1, TextStyle::plain([colour[0], colour[1], colour[2], 1.0])),
                    ),
                    Swatch::Solid(..) => spans.push(Span::new(" ", TextStyle::plain(LEGEND_TEXT))),
                }
                spans.push(Span::new(
                    format!(" {}", entry.text),
                    TextStyle::plain(LEGEND_TEXT),
                ));
                at = column + entry.cells();
            }
            if more > 0 && index + 1 == count {
                spans.push(Span::new(format!("  +{more}"), TextStyle::plain(DIM)));
            }
            Row::new(crate::ui_text::cut(&spans, columns))
        })
        .collect()
}

/// The small solids of the legend of `mesh`, kept with it, on the face of `panel`, sized as `fit` says, drawn with
/// `face`, each beside its kind's name, as a node of that kind is drawn.
pub fn legend_icons(panel: &Panel, mesh: &Mesh, fit: Fit, face: Mat4, out: &mut Vec<Instance>) {
    let diagram = mesh.source();
    let (columns, body) = self::fit(panel, diagram, fit);
    let (width, height) = super::sized(columns, body);
    let (_, most, _) = shape(panel, diagram, fit);
    let widths: Vec<usize> = mesh.legend.iter().map(Entry::cells).collect();
    let (rows, _) = legend_rows(&widths, columns.saturating_sub(4), most);
    // The face's last rows, below its heading's three and its body's others.
    let first = 3 + body - rows.len();
    let (scale, rotation, _) = face.to_scale_rotation_translation();
    let size = CELL_HEIGHT as f32 * PIXEL * 0.8;
    for (index, row) in rows.iter().enumerate() {
        for (column, entry) in row {
            let Swatch::Solid(solid, shape, turned) = mesh.legend[*entry].swatch else {
                continue;
            };
            let at = Vec3::new(
                -width / 2.0
                    + (PADDING.0 as f32 + (*column as f32 + 0.5) * CELL_WIDTH as f32) * PIXEL,
                height / 2.0
                    - (PADDING.1 as f32 + ((first + index) as f32 + 0.5) * CELL_HEIGHT as f32)
                        * PIXEL,
                size / 2.0 + STAND_OFF,
            );
            out.push(Instance {
                solid,
                position: face.transform_point3(at),
                scale: shape * size * scale,
                basis: Mat3::from_quat(rotation) * turned,
                colour: LEGEND_SOLID,
            });
        }
    }
}

/// How many columns and rows of body a volume's face has, sized as `fit` says: a slot's, or, floating, as large as its
/// diagram, its longest side within [`FLOATING`], and never narrower than its title, with room above it for its labels
/// and its legend's rows below.
pub(super) fn fit(panel: &Panel, diagram: &Diagram, fit: Fit) -> (usize, usize) {
    let (columns, most, rows) = shape(panel, diagram, fit);
    match fit {
        Fit::Slot(cells) => (cells.columns, cells.body),
        Fit::Text => (
            columns,
            rows + legend_height(&legend_widths(diagram), columns, most),
        ),
    }
}

/// How wide and tall a volume of `diagram` floats, in metres, at its own scale: its longest side within [`FLOATING`].
/// Its plate grows by as much as its front, nearer the eyes, is seen larger from [`FLOATING_READ`] away, so that the
/// diagram drawn within it is as large.
fn floating_size(diagram: &Diagram) -> (f32, f32) {
    let extent = diagram.extent.map(|side| f32::from(side.max(1)) / 1000.0);
    let longest = extent.iter().copied().fold(0.0, f32::max);
    let scale = longest.clamp(FLOATING.0, FLOATING.1) / longest;
    let nearer = FLOATING_READ / (FLOATING_READ - (extent[2] * scale).min(FLOATING_READ / 3.0));
    // No more than its rows may grow, so that it does not grow only wider.
    let tallest = (FIT_ROWS.1 * CELL_HEIGHT) as f32 * PIXEL;
    let nearer = nearer.min((tallest / (extent[1] * scale)).max(1.0));
    (extent[0] * scale * nearer, extent[1] * scale * nearer)
}

/// A volume's face, sized as `fit` says, framed by `border`: its title, bold, its metadata and age, a rule, the plate its
/// diagram stands in front of, dark in a slot and faint floating, and its legend at its foot.
pub(super) fn face(panel: &Panel, diagram: &Diagram, fit: Fit, border: Option<[f32; 4]>) -> Canvas {
    let (columns, body) = self::fit(panel, diagram, fit);
    let mut canvas = Canvas::new(columns, PIXEL);
    canvas.padding = PADDING;
    canvas.background = match fit {
        Fit::Slot(_) => [0.015, 0.025, 0.045, 1.0],
        Fit::Text => [0.02, 0.035, 0.06, 0.3],
    };
    canvas.border = border;
    canvas.rows = vec![
        Row::text(&panel.title, TextStyle::bold(BRIGHT), columns),
        Row::text(&super::about(panel), TextStyle::plain(DIM), columns),
        Mark::Rule.row(columns, TextStyle::plain(DIM)),
    ];
    let (_, most, _) = shape(panel, diagram, fit);
    let legend = legend_lines(&legend(diagram), columns, most);
    canvas.rows.extend(std::iter::repeat_n(
        Row::new(Vec::new()),
        body.saturating_sub(legend.len()),
    ));
    canvas.rows.extend(legend);
    canvas
}

/// Where a volume of `diagram` stands on its face, sized as `fit` says: from its own millimetres into the face's own
/// metres, its middle in the middle of the plate between the room below its heading kept for its labels and its
/// legend, as large as fits there, no deeper than half its height in a slot, and its back just in front of the face.
pub fn placed(panel: &Panel, diagram: &Diagram, fit: Fit) -> Mat4 {
    let (columns, body) = self::fit(panel, diagram, fit);
    let (_, most, _) = shape(panel, diagram, fit);
    let (width, height) = super::sized(columns, body);
    let legend = legend_height(&legend_widths(diagram), columns, most);
    let room = LABEL_ROOM.min(body.saturating_sub(legend + 1));
    let body = body.saturating_sub(legend + room).max(1);
    let area = (
        (columns * CELL_WIDTH) as f32 * PIXEL,
        (body * CELL_HEIGHT) as f32 * PIXEL,
    );
    let top = height / 2.0 - (PADDING.1 + (3 + room) * CELL_HEIGHT) as f32 * PIXEL;
    let left = -width / 2.0 + PADDING.0 as f32 * PIXEL;
    let middle = Vec3::new(left + area.0 / 2.0, top - area.1 / 2.0, 0.0);
    let extent = diagram.extent.map(|side| f32::from(side.max(1)));
    // As large as fits, its front, nearer the eyes than the face, seen within the area from as far as it is read,
    // straight on, so that nothing of it is seen over its title or its legend: where its front, `depth` before the
    // face, seen from `read` away, grows by `read / (read - depth)`.
    let read = match fit {
        Fit::Slot(_) => SLOT_READ,
        Fit::Text => FLOATING_READ,
    };
    let fits = |area: f32, side: f32| area * read / (side * read + area * extent[2]);
    let mut scale = fits(area.0, extent[0]).min(fits(area.1, extent[1]));
    if matches!(fit, Fit::Slot(_)) {
        scale = scale.min(area.1 / 2.0 / extent[2]);
    }
    let depth = extent[2] * scale;
    Mat4::from_translation(middle + Vec3::Z * (depth / 2.0 + STAND_OFF))
        * Mat4::from_scale(Vec3::splat(scale))
}

/// A volume as a frame draws it: what it draws, kept across frames, from its own millimetres into the scene, the node
/// the participant focuses on, how far into the flows' movement it is, and where the viewer is, whose distance its
/// leaders keep their width for. See `design/diagrams.md`.
#[derive(Clone, Debug)]
pub struct Drawn {
    pub mesh: Arc<Mesh>,
    pub model: Mat4,
    pub focus: Option<usize>,
    pub time: f32,
    pub viewer: Vec3,
    /// The small solids of its legend, in the scene, on its face.
    pub legend: Vec<Instance>,
}

impl PartialEq for Drawn {
    /// Two are alike where they draw the same mesh alike.
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.mesh, &other.mesh)
            && self.model == other.model
            && self.focus == other.focus
            && self.time == other.time
            && self.viewer == other.viewer
            && self.legend == other.legend
    }
}

impl Drawn {
    /// Its solids in the scene.
    pub fn instances(&self, out: &mut Vec<Instance>) {
        self.mesh
            .instances(self.model, self.focus, self.time, self.viewer, out);
        out.extend_from_slice(&self.legend);
    }
}

mod leaders;
pub mod springs;

#[cfg(test)]
mod tests;
