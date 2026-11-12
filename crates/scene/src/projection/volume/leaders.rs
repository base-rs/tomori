// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! A volume's world edges, drawn in the space rather than in the volume: each a leader line from its node to the thing
//! in the world it points at, where the server last said that thing stands, curving out of the face of the volume
//! nearest the thing and coming down onto it from above, as a finger points, in the edge's style, with its arrowheads;
//! and a mark on the thing, a ring about an object or on the floor about an anchor, a halo above a person, a worker or
//! the agent, or a panel's or wall's frame lit, with a label at the thing saying which step points there and why. They
//! are faint unless the participant focuses on their step, or the volume has few, so that lines across the space stay
//! quiet. See `design/diagrams.md`.
use std::sync::Arc;

use glam::{Mat3, Mat4, Quat, Vec3};
use tomori_protocol::diagram::{Arrow, Diagram, Style};
use tomori_protocol::world::{Kind, Mark};

use super::{BRIGHT, EDGE, Instance, Solid, along, label, passing, path_length, point_along, rgb};
use crate::canvas::Canvas;

/// A leader's radius at a metre from the viewer, in metres, growing with distance so that it keeps its angular size.
const RADIUS: f32 = 0.0015;
/// How many straight pieces a leader's curve is drawn with, and a ring or halo.
const SEGMENTS: usize = 16;
const RING_SEGMENTS: usize = 20;
/// The least radius of a ring or halo, how far above what it marks a halo stands, and how far above a mark a label.
const LEAST_RING: f32 = 0.08;
const HALO_ABOVE: f32 = 0.1;
const LABEL_ABOVE: f32 = 0.1;
/// How steeply a leader comes down onto what it points at, in degrees above the horizontal.
const ARRIVING: f32 = 30.0;
/// A dashed leader's dashes and gaps, a dotted one's spacing, a flow's beads, how fast they move and how many, and an
/// arrowhead's radius and length, in metres.
const DASH: (f32, f32) = (0.04, 0.03);
const DOT: f32 = 0.035;
const BEAD: f32 = 0.006;
const FLOW: f32 = 0.25;
const BEADS: usize = 4;
const CONE: (f32, f32) = (0.008, 0.025);
/// The most dashes or dots a leader is drawn with, spaced further apart along a longer one, so that a line to something
/// far off costs no more than one a few metres long.
const PIECES: f32 = 64.0;
/// How bright a leader is drawn: its volume seen alone, with more than [`FEW`] leaders, at half strength, and its mark at
/// a third; dimmed to a fifth while the participant focuses on another node.
const ALONE: (f32, f32) = (0.5, 1.0 / 3.0);
const DIMMED: f32 = 0.2;
const FEW: usize = 3;
/// A leader's label's font pixel at a metre and a half, in metres, growing with distance beyond it.
const LABEL_PIXEL: f32 = 0.0016;

/// A world edge as a volume draws it: its node, the thing it points at as the server last said, and how it is drawn.
#[derive(Clone, Debug)]
pub(super) struct Leader {
    /// The world edge it draws, by its index, and the node it comes from.
    pub edge: usize,
    pub node: usize,
    kind: Kind,
    target: Vec3,
    radius: f32,
    mark: Mark,
    /// A flat target's way it faces and its width and height.
    frame: Option<(Quat, [f32; 2])>,
    style: Style,
    direction: Arrow,
    colour: [f32; 3],
    mark_colour: [f32; 3],
    width: f32,
    pub(super) label: Arc<Canvas>,
}

impl Leader {
    /// The leaders of `diagram`'s world edges, as [`Diagram::bounded`] holds them, every edge between one of its nodes
    /// and one of its targets, to the targets it says are here.
    pub(super) fn of(diagram: &Diagram) -> Vec<Leader> {
        diagram
            .world_edges
            .iter()
            .enumerate()
            .filter_map(|(index, edge)| {
                let node = &diagram.nodes[usize::from(edge.from)];
                let target = &diagram.world[usize::from(edge.to)];
                let at = target.drawn_at()?;
                let colour = edge.colour.or(target.colour).map_or(EDGE, rgb);
                let mark_colour = target.colour.or(edge.colour).map_or(EDGE, rgb);
                let text = if edge.label.is_empty() {
                    if target.label.is_empty() {
                        target.reference.kind.name()
                    } else {
                        target.label.as_str()
                    }
                } else {
                    edge.label.as_str()
                };
                Some(Leader {
                    edge: index,
                    node: usize::from(edge.from),
                    kind: target.reference.kind,
                    target: Vec3::from_array(at),
                    radius: target.radius,
                    mark: target.mark.on(target.reference.kind),
                    frame: target
                        .frame
                        .map(|frame| (Quat::from_array(frame.orientation).normalize(), frame.size)),
                    style: edge.style,
                    direction: edge.direction,
                    colour,
                    mark_colour,
                    width: (1.0 + 0.5 * f32::from(edge.weight.saturating_sub(1)))
                        * if edge.style == Style::Thick { 2.0 } else { 1.0 },
                    label: Arc::new(label(text, Some(&node.label), BRIGHT, 0.85)),
                })
            })
            .collect()
    }

    /// Where what it points at stands, in the space.
    pub(super) fn target(&self) -> Vec3 {
        self.target
    }

    /// Where its ring or halo stands and how wide it is, if it has one.
    fn ring(&self) -> Option<(Vec3, f32)> {
        match (self.mark, self.frame) {
            (Mark::Ring, _) if self.kind == Kind::Anchor => {
                Some((self.target + Vec3::Y * 0.02, self.radius.max(LEAST_RING)))
            }
            (Mark::Ring, _) => Some((self.target, self.radius.max(LEAST_RING))),
            (Mark::Halo, _) | (Mark::Outline, None) => Some((
                self.target + Vec3::Y * (self.radius + HALO_ABOVE),
                (self.radius * 0.6).max(LEAST_RING),
            )),
            _ => None,
        }
    }

    /// The corners of its frame, if it is outlined and has one, in order about it.
    fn corners(&self) -> Option<[Vec3; 4]> {
        let (turn, [width, height]) = self.frame.filter(|_| self.mark == Mark::Outline)?;
        let corner = |x: f32, y: f32| {
            self.target + turn * Vec3::new(x * width / 2.0, y * height / 2.0, 0.01)
        };
        Some([
            corner(-1.0, 1.0),
            corner(1.0, 1.0),
            corner(1.0, -1.0),
            corner(-1.0, -1.0),
        ])
    }

    /// Where it ends, coming from `from`: on its ring or halo, on the side nearer `from`; at the top of its frame, nearest
    /// `from`; or else just above what it points at.
    fn end(&self, from: Vec3) -> Vec3 {
        if let Some((middle, radius)) = self.ring() {
            let towards = Vec3::new(from.x - middle.x, 0.0, from.z - middle.z)
                .try_normalize()
                .unwrap_or(Vec3::Z);
            return middle + towards * radius;
        }
        if let Some([left, right, ..]) = self.corners() {
            let along = right - left;
            let share =
                ((from - left).dot(along) / along.length_squared().max(1e-6)).clamp(0.0, 1.0);
            return left + along * share;
        }
        self.target + Vec3::Y * self.radius
    }

    /// Its path in the space, from `start`, its node's middle, `start_radius` across, leaving along `out`, the face of
    /// the volume nearest what it points at: a cubic curve, its first tangent along `out`, its last coming down onto its
    /// end from above at [`ARRIVING`].
    pub(super) fn path(&self, start: Vec3, start_radius: f32, out: Vec3) -> Vec<Vec3> {
        let first = start + out * start_radius;
        let last = self.end(first);
        let reach = ((last - first).length() * 0.35).clamp(0.05, 1.5);
        let back = Vec3::new(first.x - last.x, 0.0, first.z - last.z)
            .try_normalize()
            .unwrap_or(out);
        let (sin, cos) = ARRIVING.to_radians().sin_cos();
        let controls = [
            first,
            first + out * reach,
            last + (back * cos + Vec3::Y * sin) * reach,
            last,
        ];
        (0..=SEGMENTS)
            .map(|step| {
                let t = step as f32 / SEGMENTS as f32;
                let u = 1.0 - t;
                controls[0] * (u * u * u)
                    + controls[1] * (3.0 * u * u * t)
                    + controls[2] * (3.0 * u * t * t)
                    + controls[3] * (t * t * t)
            })
            .collect()
    }

    /// The ring, halo or frame it marks its target with, as a closed path, if it has one.
    pub(super) fn mark_path(&self) -> Option<Vec<Vec3>> {
        if let Some((middle, radius)) = self.ring() {
            return Some(
                (0..=RING_SEGMENTS)
                    .map(|step| {
                        let angle = std::f32::consts::TAU * step as f32 / RING_SEGMENTS as f32;
                        middle + Vec3::new(angle.cos(), 0.0, angle.sin()) * radius
                    })
                    .collect(),
            );
        }
        self.corners()
            .map(|corners| corners.iter().chain(&corners[..1]).copied().collect())
    }

    /// Where its label stands: above its end, coming from `from`.
    pub(super) fn label_at(&self, from: Vec3) -> Vec3 {
        self.end(from) + Vec3::Y * LABEL_ABOVE
    }

    /// Its label, as a frame draws it at `at`, facing `viewer`, larger the further it is.
    pub(super) fn sheet(&self, at: Vec3, viewer: Vec3) -> crate::controller_ui::Sheet {
        let pixel = LABEL_PIXEL * ((at - viewer).length() / 1.5).max(1.0);
        crate::controller_ui::Sheet {
            canvas: Some(self.label.clone()),
            model: Some(crate::recording::facing(
                at,
                Vec3::new(viewer.x, at.y, viewer.z),
                pixel / super::PIXEL,
            )),
        }
    }

    /// Its solids along `path`, seen from `viewer`, `time` seconds into a flow's movement, at `strength` and its mark at
    /// `marked`: its line in its style with its arrowheads, a flow's beads, and its mark.
    pub(super) fn instances(
        &self,
        path: &[Vec3],
        viewer: Vec3,
        time: f32,
        (strength, marked): (f32, f32),
        out: &mut Vec<Instance>,
    ) {
        let colour = self.colour.map(|channel| channel * strength);
        let radius = |at: Vec3| RADIUS * self.width * (at - viewer).length().max(1.0);
        let tube =
            |from: Vec3, to: Vec3, radius: f32, colour: [f32; 3], out: &mut Vec<Instance>| {
                out.push(Instance {
                    solid: Solid::Tube,
                    position: (from + to) / 2.0,
                    scale: Vec3::new(radius * 2.0, (to - from).length(), radius * 2.0),
                    basis: along(to - from),
                    colour,
                });
            };
        let length = path_length(path);
        match self.style {
            Style::Dashed => {
                // At most [`PIECES`] dashes, each as long against its gap as near.
                let stretch = (length / (PIECES * (DASH.0 + DASH.1))).max(1.0);
                let (dash, gap) = (DASH.0 * stretch, DASH.1 * stretch);
                let mut at = 0.0;
                while at < length {
                    let (from, to) = (
                        point_along(path, at),
                        point_along(path, (at + dash).min(length)),
                    );
                    tube(from, to, radius(from), colour, out);
                    at += dash + gap;
                }
            }
            Style::Dotted => {
                let spacing = DOT.max(length / PIECES);
                let mut at = spacing / 2.0;
                while at < length {
                    let point = point_along(path, at);
                    out.push(Instance {
                        solid: Solid::Diamond,
                        position: point,
                        scale: Vec3::splat(radius(point) * 3.0),
                        basis: Mat3::IDENTITY,
                        colour,
                    });
                    at += spacing;
                }
            }
            _ => {
                for pair in path.windows(2) {
                    tube(pair[0], pair[1], radius(pair[0]), colour, out);
                }
            }
        }
        let last = path.len() - 1;
        let (head, tail) = match self.direction {
            Arrow::Forward => (true, false),
            Arrow::Back => (false, true),
            Arrow::Both => (true, true),
            _ => (false, false),
        };
        let ends = [
            (head, path[last], path[last] - path[last - 1]),
            (tail, path[0], path[0] - path[1]),
        ];
        for (wanted, tip, way) in ends {
            let way = way.try_normalize().unwrap_or(Vec3::Y);
            if wanted {
                let scale = (tip - viewer).length().max(1.0) * self.width;
                out.push(Instance {
                    solid: Solid::Cone,
                    position: tip - way * CONE.1 * scale / 2.0,
                    scale: Vec3::new(CONE.0 * 2.0, CONE.1, CONE.0 * 2.0) * scale,
                    basis: along(way),
                    colour,
                });
            }
        }
        if self.style == Style::Flow {
            // Towards the arrowhead: from the node, unless it points back at it.
            let bright = colour.map(|channel| (channel * 1.6).min(1.0));
            for bead in 0..BEADS {
                let share = (time * FLOW / length.max(0.01) + bead as f32 / BEADS as f32).fract();
                let share = if self.direction == Arrow::Back {
                    1.0 - share
                } else {
                    share
                };
                let point = point_along(path, share * length);
                out.push(Instance {
                    solid: Solid::Sphere,
                    position: point,
                    scale: Vec3::splat((BEAD + radius(point)) * 2.0),
                    basis: Mat3::IDENTITY,
                    colour: bright,
                });
            }
        }
        if let Some(mark) = self.mark_path() {
            let colour = self.mark_colour.map(|channel| channel * marked);
            for pair in mark.windows(2) {
                tube(pair[0], pair[1], radius(pair[0]) * 1.5, colour, out);
            }
        }
    }
}

/// How bright `leaders`' leader `index` is drawn, its line and its mark, while the participant focuses on the node
/// `focus`: in full, for one of its own; dimmed, for another's; and without a focus, in full where there are [`FEW`] or
/// fewer, or else at [`ALONE`].
pub(super) fn strength(leaders: &[Leader], index: usize, focus: Option<usize>) -> (f32, f32) {
    match focus {
        Some(node) if leaders[index].node == node => (1.0, 1.0),
        Some(_) => (DIMMED, DIMMED),
        None if leaders.len() <= FEW => (1.0, 1.0),
        None => ALONE,
    }
}

/// The way a leader to `target` leaves a volume drawn with `model` of `extent`, its width, height and depth in its own
/// millimetres: out of the face of its box nearest `target`, in the space.
pub(super) fn out_of(model: Mat4, extent: [u16; 3], target: Vec3) -> Vec3 {
    let local = model.inverse().transform_point3(target);
    let half = extent.map(|side| f32::from(side.max(1)) / 2.0);
    let axis = (0..3)
        .max_by(|a, b| (local[*a].abs() / half[*a]).total_cmp(&(local[*b].abs() / half[*b])))
        .unwrap_or(2);
    let mut normal = Vec3::ZERO;
    normal[axis] = if local[axis] < 0.0 { -1.0 } else { 1.0 };
    model
        .transform_vector3(normal)
        .try_normalize()
        .unwrap_or(Vec3::Z)
}

/// How far along `ray` it passes within reach of `path`, if it does: within 1.5 cm or half a degree, as the volume's
/// own edges are met.
pub(super) fn met(path: &[Vec3], ray: &crate::pointing::Ray) -> Option<f32> {
    path.windows(2)
        .filter_map(|pair| {
            let (distance, apart) = passing(ray.origin, ray.direction, pair[0], pair[1]);
            (apart <= super::EDGE_REACH.0.max(super::EDGE_REACH.1 * distance)).then_some(distance)
        })
        .reduce(f32::min)
}
