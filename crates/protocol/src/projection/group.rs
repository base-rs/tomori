// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Walls snapped together into a group, which reads as one board: where a wall stands attached to another's edge, when
//! a wall carried is near enough to another's edge to snap to it, whether two walls overlap, and which edges of a group's
//! walls are joined. The server, which keeps groups and checks snaps, and every client, which previews a snap and draws a
//! group as one, share it, so that the two cannot drift apart. See `design/projection.md`.
//!
//! A wall's **surface** is the plane it stands in, or, curved, the upright cylinder its slots stand along, about the
//! line its arc curves about. A point on it is `u` metres to the wall's right, as seen from in front, measured along the
//! surface, and `y` metres up, from the wall's pose; a wall's slots span its [`Extent`] there. Two walls share a surface
//! when they stand in the same plane, facing the same way, or along the same arc, about the same line at the same
//! radius; only such walls snap together.
use serde::{Deserialize, Serialize};

use super::Wall;
use super::layout::{SlotFrame, multiply, rotate};
use crate::Pose;

/// How near a wall carried brings one of its edges to where it would snap beside another's, across the edge, in metres,
/// at least, to snap: generous, as a magnet is, since the snap lines it up exactly.
pub const SNAP_REACH: f32 = 0.30;

/// How near, as an angle seen from the viewer, a wall carried brings its edge to where it would snap, in radians, where
/// that is more than [`SNAP_REACH`]: 6°, so that a wall carried far away snaps as readily as one near.
pub const SNAP_ANGLE: f32 = 6.0 * std::f32::consts::PI / 180.0;

/// How far in front of or behind the other's surface a wall carried may be, in metres, to snap to it: less than a firm
/// pull takes a wall out of its group, so that one pulled out is not snapped straight back.
pub const SNAP_DEPTH: f32 = 0.25;

/// How far a wall carried may be turned, about the vertical, from facing as the other's surface faces where it would
/// stand, in radians, to snap to it: 30°, three of the stick's turns, so that turning it never keeps it from snapping.
pub const SNAP_TURN: f32 = 30.0 * std::f32::consts::PI / 180.0;

/// How far a wall carried may be tilted from the other's upright, in radians, to snap to it: 15°.
pub const SNAP_TILT: f32 = 15.0 * std::f32::consts::PI / 180.0;

/// How much wider a snap already shown may be held than it is first shown, so that it does not flicker at the edge of
/// reach: a quarter.
pub const SNAP_HOLD: f32 = 1.25;

/// How much nearer, in metres, a snap across a side ranks than one across the top or bottom, so that walls go side by
/// side unless one is clearly above or below the other.
const SIDE_BIAS: f32 = 0.05;

/// How much a snap ranks further for each radian a wall carried turns to line up, in metres.
const TURN_COST: f32 = 0.2;

/// How far apart two walls' planes may be, or the lines two curved walls curve about, in metres, for them to share a
/// surface, as walls of a group do and as overlapping walls must.
pub const SURFACE_DEPTH: f32 = 0.05;

/// How far apart the ways two flat walls face may be, in radians, for them to share a surface: 5°.
pub const SURFACE_TURN: f32 = 5.0 * std::f32::consts::PI / 180.0;

/// How far apart two curved walls' radii may be, as a proportion of the radius, for them to share a surface.
pub const SNAP_RADIUS: f32 = 0.01;

/// How near the gap between two walls' slots is to one gutter, and how far they must meet along an edge, in metres, for
/// the edge to be joined.
pub const JOIN_TOLERANCE: f32 = 0.01;

/// How far into each other two walls' slots may reach, in metres, before they overlap.
const OVERLAP: f32 = 0.001;

/// An edge of a wall, as seen from in front of it. `Unknown` stands for one a newer peer sends.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Edge {
    Left,
    Right,
    Top,
    Bottom,
    #[serde(other)]
    Unknown,
}

impl Edge {
    /// Every edge a wall has.
    pub const ALL: [Edge; 4] = [Edge::Left, Edge::Right, Edge::Top, Edge::Bottom];

    /// The edge facing it on a wall across it.
    pub fn opposite(self) -> Edge {
        match self {
            Edge::Left => Edge::Right,
            Edge::Right => Edge::Left,
            Edge::Top => Edge::Bottom,
            Edge::Bottom => Edge::Top,
            Edge::Unknown => Edge::Unknown,
        }
    }

    /// The edge `name` names, in any case: `left`, `right`, `top` or `bottom`, or `above` and `below` for the last two.
    pub fn parse(name: &str) -> Option<Edge> {
        match name.trim().to_ascii_lowercase().as_str() {
            "left" => Some(Edge::Left),
            "right" => Some(Edge::Right),
            "top" | "above" => Some(Edge::Top),
            "bottom" | "below" => Some(Edge::Bottom),
            _ => None,
        }
    }

    /// Whether it is a side, so that walls across it stand side by side.
    fn side(self) -> bool {
        matches!(self, Edge::Left | Edge::Right)
    }
}

/// How a wall attached to an edge is aligned along it: its top, or its left, with the edge's (`Start`), its middle with
/// the edge's middle, or its bottom, or its right, with the edge's (`End`). `Unknown` stands for one a newer peer sends.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Align {
    Start,
    Middle,
    End,
    #[serde(other)]
    Unknown,
}

impl Align {
    /// Every alignment.
    pub const ALL: [Align; 3] = [Align::Start, Align::Middle, Align::End];

    /// The alignment `name` names, in any case: `start`, `middle` or `end`, or `top`, `left`, `centre`, `center`,
    /// `bottom` and `right` for them.
    pub fn parse(name: &str) -> Option<Align> {
        match name.trim().to_ascii_lowercase().as_str() {
            "start" | "top" | "left" => Some(Align::Start),
            "middle" | "centre" | "center" => Some(Align::Middle),
            "end" | "bottom" | "right" => Some(Align::End),
            _ => None,
        }
    }
}

/// Where a wall is attached in its group: beside the wall `to`, across its `edge`, aligned along it as `align` says.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Attach {
    pub to: String,
    pub edge: Edge,
    pub align: Align,
}

impl Attach {
    pub fn new(to: impl Into<String>, edge: Edge, align: Align) -> Self {
        Self {
            to: to.into(),
            edge,
            align,
        }
    }

    /// Whether it names an edge and an alignment this peer knows.
    pub fn known(&self) -> bool {
        self.edge != Edge::Unknown && self.align != Align::Unknown
    }
}

/// One of a wall's edges joined to another wall of its group, which stands across it: a client leaves its border and
/// outer margin off where they meet, so that the group reads as one board.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Join {
    pub edge: Edge,
    pub wall: String,
}

impl Join {
    pub fn new(edge: Edge, wall: impl Into<String>) -> Self {
        Self {
            edge,
            wall: wall.into(),
        }
    }
}

/// Where a wall's slots reach on its surface, from its pose, in metres: their left and right edges, along the surface,
/// and their bottom and top.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct Extent {
    pub left: f32,
    pub right: f32,
    pub bottom: f32,
    pub top: f32,
}

impl Extent {
    pub fn new(left: f32, right: f32, bottom: f32, top: f32) -> Self {
        Self {
            left,
            right,
            bottom,
            top,
        }
    }

    /// It, moved `u` to the right and `y` up.
    pub fn shifted(&self, u: f32, y: f32) -> Extent {
        Extent {
            left: self.left + u,
            right: self.right + u,
            bottom: self.bottom + y,
            top: self.top + y,
        }
    }

    /// Where it reaches across `edge`, along the surface for a side and up it otherwise.
    fn at(&self, edge: Edge) -> f32 {
        match edge {
            Edge::Left => self.left,
            Edge::Right => self.right,
            Edge::Top => self.top,
            _ => self.bottom,
        }
    }

    /// From where to where it reaches along `edge`: up it for a side, and along the surface otherwise.
    fn along(&self, edge: Edge) -> (f32, f32) {
        if edge.side() {
            (self.bottom, self.top)
        } else {
            (self.left, self.right)
        }
    }
}

/// The unit quaternion `rotation`, inverted.
fn inverse([x, y, z, w]: [f32; 4]) -> [f32; 4] {
    [-x, -y, -z, w]
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

/// `angle`, in radians, within half a turn either way.
fn wrapped(angle: f32) -> f32 {
    let turn = std::f32::consts::TAU;
    angle - turn * ((angle + turn / 2.0) / turn).floor()
}

/// The pose `u` metres to the right of `base` along its surface, curved at `radius` or flat, and `y` metres above it,
/// facing as the surface faces there.
pub fn along(base: &Pose, radius: Option<f32>, u: f32, y: f32) -> Pose {
    let frame = |position: [f32; 3], yaw: f32| SlotFrame {
        position,
        yaw,
        scale: 1.0,
        size: [0.0, 0.0],
    };
    match radius {
        None => frame([u, y, 0.0], 0.0).pose(base),
        Some(radius) => {
            let angle = u / radius;
            let (sin, cos) = angle.sin_cos();
            frame([radius * sin, y, radius * (1.0 - cos)], -angle).pose(base)
        }
    }
}

/// Where `pose` stands on the surface of `base`, curved at `radius` or flat: how far to its right along it and how far up,
/// how far in front of it, towards the line it curves about, and how far it is turned, about the vertical, from facing as
/// the surface faces there, in radians.
pub fn on_surface(base: &Pose, radius: Option<f32>, pose: &Pose) -> (f32, f32, f32, f32) {
    let turn = inverse(base.orientation);
    let [x, y, z] = rotate(turn, sub(pose.position, base.position));
    let [ahead_x, _, ahead_z] = rotate(turn, rotate(pose.orientation, [0.0, 0.0, 1.0]));
    let yaw = ahead_x.atan2(ahead_z);
    match radius {
        None => (x, y, z, wrapped(yaw)),
        Some(radius) => {
            let (across, back) = (x, z - radius);
            let angle = across.atan2(-back);
            (
                angle * radius,
                y,
                radius - across.hypot(back),
                wrapped(yaw + angle),
            )
        }
    }
}

/// Where `other` stands once the motion that takes `from` to `to` moves it with them, as a group's walls are carried
/// together: turned as they are turned, about the vertical through `from`, and moved as it moves.
pub fn carried_with(from: &Pose, to: &Pose, other: &Pose) -> Pose {
    let turn = multiply(to.orientation, inverse(from.orientation));
    let [x, y, z] = rotate(turn, sub(other.position, from.position));
    let length = turn
        .iter()
        .map(|value| value * value)
        .sum::<f32>()
        .sqrt()
        .max(1e-6);
    let orientation = multiply(turn.map(|value| value / length), other.orientation);
    Pose {
        position: [to.position[0] + x, to.position[1] + y, to.position[2] + z],
        orientation,
    }
}

/// Where the line a wall curved at `radius` about, standing at `pose`, curves about, level with its pose.
fn centre(pose: &Pose, radius: f32) -> [f32; 3] {
    let ahead = rotate(pose.orientation, [0.0, 0.0, radius]);
    [
        pose.position[0] + ahead[0],
        pose.position[1],
        pose.position[2] + ahead[2],
    ]
}

/// Whether `a` and `b` share a surface, as walls of a group do: both flat, `b`'s middle within `depth` metres of `a`'s
/// plane and facing the same way within [`SURFACE_TURN`]; or both curved, at radii within [`SNAP_RADIUS`] of each other,
/// about lines within `depth` of each other.
fn sharing(a: &Wall, b: &Wall, depth: f32) -> bool {
    match (a.radius(), b.radius()) {
        (None, None) => {
            let (_, _, off, turn) = on_surface(&a.pose, None, &b.pose);
            off.abs() <= depth && turn.abs() <= SURFACE_TURN
        }
        (Some(ra), Some(rb)) => {
            let (ca, cb) = (centre(&a.pose, ra), centre(&b.pose, rb));
            (ra - rb).abs() <= SNAP_RADIUS * ra && (ca[0] - cb[0]).hypot(ca[2] - cb[2]) <= depth
        }
        _ => false,
    }
}

/// Whether `a` and `b` share a surface, within [`SURFACE_DEPTH`], as walls standing together do and as walls whose slots
/// overlap must.
pub fn same_surface(a: &Wall, b: &Wall) -> bool {
    sharing(a, b, SURFACE_DEPTH)
}

/// Whether a wall carried may snap to `wall`: one someone added, since the Tomori wall and an environment's walls stand
/// where Tomori or the environment puts them, not one locked in place, and not one tilted, since a group's walls stand
/// upright, so that the server and every client agree.
pub fn snaps_to(wall: &Wall) -> bool {
    wall.kind == super::WallKind::Added && !wall.is_locked() && wall.tilt() == 0.0
}

/// The gap between the slots of `a` and `b` standing edge to edge: one gutter, at the larger of their two, so that
/// spacing is even across the join.
pub fn gap_between(a: &Wall, b: &Wall) -> f32 {
    a.gap().max(b.gap())
}

/// Where `member` stands, attached to `target` across its `edge` and aligned along it as `align` says, on `target`'s
/// surface, from `target`'s pose: how far to the right, and how far up. Their slots are a gap apart, as
/// [`gap_between`] says. `None` for an edge or alignment this peer does not know.
pub fn attached_offset(
    target: &Wall,
    member: &Wall,
    edge: Edge,
    align: Align,
) -> Option<(f32, f32)> {
    let gap = gap_between(target, member);
    let (to, own) = (target.extent(), member.extent());
    let aligned = |(low, high): (f32, f32), (own_low, own_high): (f32, f32)| match align {
        // A side's start is its top, an edge across's its left.
        Align::Start if edge.side() => Some(high - own_high),
        Align::End if edge.side() => Some(low - own_low),
        Align::Start => Some(low - own_low),
        Align::End => Some(high - own_high),
        Align::Middle => Some((low + high - own_low - own_high) / 2.0),
        Align::Unknown => None,
    };
    let along = aligned(to.along(edge), own.along(edge))?;
    Some(match edge {
        Edge::Right => (to.right + gap - own.left, along),
        Edge::Left => (to.left - gap - own.right, along),
        Edge::Top => (along, to.top + gap - own.bottom),
        Edge::Bottom => (along, to.bottom - gap - own.top),
        Edge::Unknown => return None,
    })
}

/// The pose `member` takes attached to `target` across its `edge`, aligned as `align` says, as [`attached_offset`] puts
/// it, along `target`'s surface.
pub fn attached_pose(target: &Wall, member: &Wall, edge: Edge, align: Align) -> Option<Pose> {
    let (u, y) = attached_offset(target, member, edge, align)?;
    Some(along(&target.pose, target.radius(), u, y))
}

/// How far from where it would snap a wall carried may be to snap there: across the edge, in front of or behind the
/// other's surface, turned about the vertical and tilted, in metres and radians.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct Leeway {
    pub reach: f32,
    pub depth: f32,
    pub turn: f32,
    pub tilt: f32,
}

impl Leeway {
    /// The leeway for a wall carried near the viewer: [`SNAP_REACH`], [`SNAP_DEPTH`], [`SNAP_TURN`] and [`SNAP_TILT`].
    pub const NEAR: Leeway = Leeway {
        reach: SNAP_REACH,
        depth: SNAP_DEPTH,
        turn: SNAP_TURN,
        tilt: SNAP_TILT,
    };

    /// The leeway for a wall carried `distance` metres from the viewer: its reach as wide as [`SNAP_ANGLE`] there, where
    /// that is more than [`SNAP_REACH`].
    pub fn at(distance: f32) -> Leeway {
        Leeway {
            reach: SNAP_REACH.max(distance.max(0.0) * SNAP_ANGLE.tan()),
            ..Leeway::NEAR
        }
    }

    /// It, held [`SNAP_HOLD`] wider, as a snap already shown is, so that it does not flicker at the edge.
    pub fn held(self) -> Leeway {
        Leeway {
            reach: self.reach * SNAP_HOLD,
            depth: self.depth * SNAP_HOLD,
            turn: self.turn * SNAP_HOLD,
            tilt: self.tilt * SNAP_HOLD,
        }
    }

    /// It, at most `depth` in front of or behind the other's surface.
    pub fn within(self, depth: f32) -> Leeway {
        Leeway {
            depth: self.depth.min(depth),
            ..self
        }
    }
}

/// Where a wall carried snaps to another: across which of the other's edges, aligned how, where it then stands, lined up
/// with the other, how far it is now from there, in metres, how far it turns to line up, in radians, and how it ranks
/// among others, nearest first. Snapped to a curved wall, it is curved at the other's `radius`, continuing its arc.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Snap {
    pub edge: Edge,
    pub align: Align,
    pub pose: Pose,
    pub distance: f32,
    pub turn: f32,
    pub rank: f32,
    pub radius: Option<f32>,
}

/// Where `carried`, standing where it is carried, snaps to `target`, if it does, within [`Leeway::NEAR`], as
/// [`snap_within`] says.
pub fn snap(carried: &Wall, target: &Wall) -> Option<Snap> {
    snap_within(carried, target, Leeway::NEAR)
}

/// Where `carried`, standing where it is carried, snaps to `target`, if it does, within `leeway`: when both are flat, or
/// both curved, whatever their radii, and `carried` is within `leeway`'s depth of `target`'s surface, turned and tilted
/// from facing as it faces there within its turn and tilt, and one of its edges within its reach of where it would stand
/// attached across the facing edge of `target`, the two meeting along it, or nearly, aligned as is nearest where it is
/// now. It then stands lined up with `target`: in its plane, facing as it faces, or, curved, at its radius about the same
/// line, so that the participant need not line it up first. Of `target`'s edges, the one that ranks first: the nearest,
/// a side before the top or bottom unless one is clearly nearer.
pub fn snap_within(carried: &Wall, target: &Wall, leeway: Leeway) -> Option<Snap> {
    // A locked wall is carried nowhere, nor snapped to.
    if carried.is_locked() || target.is_locked() {
        return None;
    }
    let radius = target.radius();
    // Curved alike, it takes the other's radius, so that the two read as one curve.
    let shaped = match (radius, carried.radius()) {
        (None, None) => carried.clone(),
        (Some(radius), Some(_)) => carried.clone().with_shape(super::WallShape::Arc { radius }),
        _ => return None,
    };
    if shaped.radius() != radius {
        // Curved that tightly, it would go round more than half a circle.
        return None;
    }
    let (u, y, depth, turn) = on_surface(&target.pose, radius, &carried.pose);
    let up = rotate(
        inverse(target.pose.orientation),
        rotate(carried.pose.orientation, [0.0, 1.0, 0.0]),
    );
    let tilt = up[1].clamp(-1.0, 1.0).acos();
    if depth.abs() > leeway.depth || turn.abs() > leeway.turn || tilt > leeway.tilt {
        return None;
    }
    let to = target.extent();
    let at = shaped.extent().shifted(u, y);
    Edge::ALL
        .into_iter()
        .filter_map(|edge| {
            // They meet along the edge, or nearly.
            let ((low, high), (own_low, own_high)) = (to.along(edge), at.along(edge));
            if own_high.min(high) - own_low.max(low) <= -leeway.reach {
                return None;
            }
            Align::ALL
                .into_iter()
                .filter_map(|align| {
                    attached_offset(target, &shaped, edge, align).map(|(du, dy)| {
                        let (across, along) = if edge.side() {
                            (u - du, y - dy)
                        } else {
                            (y - dy, u - du)
                        };
                        (align, du, dy, along.abs(), across)
                    })
                })
                .filter(|(.., across)| across.abs() <= leeway.reach)
                .min_by(|a, b| a.3.total_cmp(&b.3))
                .map(|(align, du, dy, along, across)| {
                    let distance = across.hypot(along).hypot(depth);
                    let bias = if edge.side() { 0.0 } else { SIDE_BIAS };
                    Snap {
                        edge,
                        align,
                        pose: along_surface(target, du, dy),
                        distance,
                        turn: turn.abs(),
                        rank: distance + TURN_COST * turn.abs() + bias,
                        radius,
                    }
                })
        })
        .min_by(|a, b| a.rank.total_cmp(&b.rank))
}

/// The pose `u` to the right of `wall` along its surface and `y` above it.
fn along_surface(wall: &Wall, u: f32, y: f32) -> Pose {
    along(&wall.pose, wall.radius(), u, y)
}

/// `b`'s slots, as they stand on `a`'s surface.
fn seen_from(a: &Wall, b: &Wall) -> Extent {
    let (u, y, _, _) = on_surface(&a.pose, a.radius(), &b.pose);
    b.extent().shifted(u, y)
}

/// Whether the slots of `a` and `b`, sharing a surface, reach into each other, as no two walls may.
pub fn overlaps(a: &Wall, b: &Wall) -> bool {
    if !same_surface(a, b) {
        return false;
    }
    let mine = a.extent();
    let theirs = seen_from(a, b);
    let up = theirs.bottom < mine.top - OVERLAP && theirs.top > mine.bottom + OVERLAP;
    // Around a curve, the other may be a whole turn either way.
    let turns = match a.radius() {
        Some(radius) => vec![
            -std::f32::consts::TAU * radius,
            0.0,
            std::f32::consts::TAU * radius,
        ],
        None => vec![0.0],
    };
    up && turns.into_iter().any(|turn| {
        theirs.left + turn < mine.right - OVERLAP && theirs.right + turn > mine.left + OVERLAP
    })
}

/// Which of `a`'s edges `b` stands across, edge to edge, their slots a gap apart, as [`attached_offset`] puts them, and
/// from where to where along it they meet, on `a`'s surface: up it for a side, and along the surface otherwise. `None`
/// unless they share a surface within [`JOIN_TOLERANCE`] and meet along it by as much.
pub fn joined(a: &Wall, b: &Wall) -> Option<(Edge, f32, f32)> {
    if !sharing(a, b, JOIN_TOLERANCE) {
        return None;
    }
    let gap = gap_between(a, b);
    let (mine, theirs) = (a.extent(), seen_from(a, b));
    Edge::ALL.into_iter().find_map(|edge| {
        let across = match edge {
            Edge::Right => theirs.left - (mine.right + gap),
            Edge::Left => (mine.left - gap) - theirs.right,
            Edge::Top => theirs.bottom - (mine.top + gap),
            _ => (mine.bottom - gap) - theirs.top,
        };
        let ((low, high), (other_low, other_high)) = (mine.along(edge), theirs.along(edge));
        let (from, to) = (low.max(other_low), high.min(other_high));
        (across.abs() <= JOIN_TOLERANCE && to - from >= JOIN_TOLERANCE).then_some((edge, from, to))
    })
}

/// How far beyond `wall`'s slots across `edge` the seam stands between it and `other`, joined there: halfway across the
/// gap between their slots.
pub fn seam(wall: &Wall, other: &Wall) -> f32 {
    gap_between(wall, other) / 2.0
}

/// Where `wall` reaches across `edge`, from its pose, on its surface.
pub fn reach(wall: &Wall, edge: Edge) -> f32 {
    wall.extent().at(edge)
}

#[cfg(test)]
mod tests;
