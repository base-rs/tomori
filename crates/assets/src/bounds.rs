// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! What a model takes up, and what the eyes see, without a GPU: boxes about what is drawn, the frustum of a view and
//! one enclosing both eyes', and whether a primitive is closed, so that its back faces need not be drawn. A renderer
//! culls what lies outside the eyes' frustum, and a test counts what is in view the same way. See
//! `design/rendering.md`.
use std::collections::HashMap;

use glam::{Mat4, Vec3, Vec4, Vec4Swizzles};

use crate::instance::Draw;
use crate::model::{Model, Primitive};

/// A box aligned with the axes, from its least corner to its greatest. One with any of its least greater than its
/// greatest, as [`Aabb::EMPTY`], holds nothing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aabb {
    pub min: Vec3,
    pub max: Vec3,
}

impl Aabb {
    /// The box that holds nothing, which any box joined to it is.
    pub const EMPTY: Self = Self {
        min: Vec3::INFINITY,
        max: Vec3::NEG_INFINITY,
    };

    /// The box from `min` to `max`.
    pub fn new(min: Vec3, max: Vec3) -> Self {
        Self { min, max }
    }

    /// The smallest box that holds `points`, or [`Aabb::EMPTY`] for none.
    pub fn from_points(points: impl IntoIterator<Item = Vec3>) -> Self {
        points
            .into_iter()
            .fold(Self::EMPTY, |bounds, point| bounds.with(point))
    }

    /// Whether it holds nothing.
    pub fn is_empty(&self) -> bool {
        self.min.cmpgt(self.max).any()
    }

    /// This box, grown to hold `point`.
    pub fn with(self, point: Vec3) -> Self {
        Self {
            min: self.min.min(point),
            max: self.max.max(point),
        }
    }

    /// The smallest box that holds this and `other`.
    pub fn union(self, other: Self) -> Self {
        Self {
            min: self.min.min(other.min),
            max: self.max.max(other.max),
        }
    }

    /// This box grown by `margin` on every side.
    pub fn grown(self, margin: f32) -> Self {
        if self.is_empty() {
            return self;
        }
        Self {
            min: self.min - Vec3::splat(margin),
            max: self.max + Vec3::splat(margin),
        }
    }

    /// The smallest box that holds this one moved by `transform`, which may turn, scale or skew it.
    pub fn transformed(self, transform: Mat4) -> Self {
        if self.is_empty() {
            return self;
        }
        // About its middle: the moved middle, and how far the moved half extents reach along each axis.
        let centre = (self.min + self.max) * 0.5;
        let half = (self.max - self.min) * 0.5;
        let moved = transform.transform_point3(centre);
        let reach = transform.x_axis.truncate().abs() * half.x
            + transform.y_axis.truncate().abs() * half.y
            + transform.z_axis.truncate().abs() * half.z;
        Self {
            min: moved - reach,
            max: moved + reach,
        }
    }

    /// Whether every corner is a number.
    pub fn is_finite(&self) -> bool {
        self.min.is_finite() && self.max.is_finite()
    }
}

/// The part of the scene a view sees: six planes, each with what is inside it on its side, `normal · point + d ≥ 0`
/// for a plane `(normal, d)`, with WebGPU's depth from 0 to 1. A frustum may enclose several views, as both eyes'.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frustum {
    planes: [Vec4; 6],
}

/// Planes that hold every point, for a frustum that culls nothing.
const EVERYWHERE: [Vec4; 6] = [Vec4::new(0.0, 0.0, 0.0, 1.0); 6];

impl Frustum {
    /// What `clip`, a view and projection taking the scene to clip space, sees, between its near and far planes; or,
    /// for a matrix that is not all numbers, everything.
    pub fn new(clip: Mat4) -> Self {
        let [x, y, z, w] = [0, 1, 2, 3].map(|row| clip.row(row));
        let planes = [w + x, w - x, w + y, w - y, z, w - z].map(|plane| {
            let length = plane.xyz().length();
            if length > f32::EPSILON && plane.is_finite() {
                plane / length
            } else {
                // A plane with no direction, such as an infinite far plane's, holds everything.
                Vec4::W
            }
        });
        Self { planes }
    }

    /// One frustum holding everything any of `clips` sees, as both eyes' together, tested once rather than once for each
    /// view: for each side, the plane of the views' that holds all their corners most tightly, moved out as far as it
    /// must be to hold them all, so that its apex lies behind the eyes, where their outer planes meet. Nothing any of the
    /// views sees is outside it. With no views, or one whose corners cannot be found, it holds everything.
    pub fn enclosing(clips: &[Mat4]) -> Self {
        // As many views as a frame has, both eyes' or one, are found without allocating, once a frame.
        const FEW: usize = 4;
        if clips.len() <= FEW {
            let mut corners = [Vec3::ZERO; 8 * FEW];
            let mut views = [Self { planes: EVERYWHERE }; FEW];
            Self::enclosing_within(
                clips,
                &mut corners[..8 * clips.len()],
                &mut views[..clips.len()],
            )
        } else {
            let mut corners = vec![Vec3::ZERO; 8 * clips.len()];
            let mut views = vec![Self { planes: EVERYWHERE }; clips.len()];
            Self::enclosing_within(clips, &mut corners, &mut views)
        }
    }

    /// [`Frustum::enclosing`], with room for `clips`' corners and their own frustums.
    fn enclosing_within(clips: &[Mat4], corners: &mut [Vec3], views: &mut [Frustum]) -> Self {
        for (index, clip) in clips.iter().enumerate() {
            let inverse = clip.inverse();
            for corner in 0..8 {
                let ndc = Vec4::new(
                    if corner & 1 == 0 { -1.0 } else { 1.0 },
                    if corner & 2 == 0 { -1.0 } else { 1.0 },
                    if corner & 4 == 0 { 0.0 } else { 1.0 },
                    1.0,
                );
                let point = inverse * ndc;
                if !point.is_finite() || point.w.abs() <= f32::EPSILON {
                    return Self { planes: EVERYWHERE };
                }
                corners[index * 8 + corner] = point.xyz() / point.w;
            }
            views[index] = Frustum::new(*clip);
        }
        if corners.is_empty() {
            return Self { planes: EVERYWHERE };
        }
        let (corners, views) = (&*corners, &*views);
        let planes = std::array::from_fn(|side| {
            views
                .iter()
                .map(|view| {
                    let plane = view.planes[side];
                    let distance = |corner: &Vec3| plane.xyz().dot(*corner) + plane.w;
                    let nearest = corners.iter().map(distance).fold(f32::INFINITY, f32::min);
                    // Out past the farthest corner outside it, and a little more for rounding.
                    let moved = plane + Vec4::W * ((-nearest).max(0.0) + 1e-4);
                    let looseness: f32 = corners
                        .iter()
                        .map(|corner| moved.xyz().dot(*corner) + moved.w)
                        .sum();
                    (moved, looseness)
                })
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map_or(Vec4::W, |(plane, _)| plane)
        });
        Self { planes }
    }

    /// Its six planes, `(normal, d)`, each holding what is inside it on the side where `normal · point + d ≥ 0`: left,
    /// right, bottom, top, near and far.
    pub fn planes(&self) -> &[Vec4; 6] {
        &self.planes
    }

    /// The same frustum in the coordinates that `transform` carries into these, as a scenery's are carried into the
    /// space's: a point `p` there is inside it exactly when `transform * p` is inside this one. Its planes are scaled
    /// as `transform` scales, which changes nothing it holds.
    pub fn transformed(&self, transform: Mat4) -> Self {
        let transpose = transform.transpose();
        Self {
            planes: self.planes.map(|plane| transpose * plane),
        }
    }

    /// Whether any of `bounds` may be seen: false only when the box is wholly outside one of its planes. A box that is
    /// not all numbers may be seen.
    pub fn sees(&self, bounds: &Aabb) -> bool {
        if bounds.is_empty() {
            return false;
        }
        if !bounds.is_finite() {
            return true;
        }
        self.planes.iter().all(|plane| {
            // The box's corner farthest along the plane's normal.
            let farthest = Vec3::select(plane.xyz().cmpge(Vec3::ZERO), bounds.max, bounds.min);
            plane.xyz().dot(farthest) + plane.w >= 0.0
        })
    }

    /// Whether `point` is inside it.
    pub fn holds(&self, point: Vec3) -> bool {
        self.planes
            .iter()
            .all(|plane| plane.xyz().dot(point) + plane.w >= 0.0)
    }
}

/// How much is drawn and how much culled, a frame or in a view: items, as a model's placed primitive or a panel, drawn
/// and culled, the draw calls made and the triangles they submit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tally {
    pub drawn: usize,
    pub culled: usize,
    pub draws: usize,
    pub triangles: usize,
}

impl std::ops::AddAssign for Tally {
    fn add_assign(&mut self, other: Self) {
        self.drawn += other.drawn;
        self.culled += other.culled;
        self.draws += other.draws;
        self.triangles += other.triangles;
    }
}

impl std::ops::Add for Tally {
    type Output = Self;

    fn add(mut self, other: Self) -> Self {
        self += other;
        self
    }
}

/// Where a primitive may be drawn, measured once: its box, and, skinned, the box of the vertices each joint of its skin
/// moves at all, in the model's space before skinning.
///
/// A skinned vertex is drawn where its joints' matrices, weighted, put it, so it lies within the box about every joint's
/// own box moved by that joint's matrix, whatever the pose: the hens' walk, flap and peck, without measuring them.
#[derive(Clone, Debug, PartialEq)]
pub struct Extent {
    local: Aabb,
    skinned: bool,
    /// Each joint's box, by its index in the skin, or none for a skin whose weights do not sum to one or are negative,
    /// whose vertices may go anywhere.
    joints: Option<Vec<Aabb>>,
}

impl Extent {
    /// How far `primitive` reaches.
    pub fn of(primitive: &Primitive) -> Self {
        let (min, max) = primitive.bounds();
        Self {
            local: Aabb::new(min, max),
            skinned: !primitive.joints.is_empty(),
            joints: joint_bounds(primitive),
        }
    }

    /// Its box, in its model's space, as it is at rest.
    pub fn local(&self) -> Aabb {
        self.local
    }

    /// Where it may be drawn in the scene as `draw` places it: its box moved by its node's matrix, or, skinned, the box
    /// about each of its joints' boxes moved by that joint's matrix; or `None` for a skin whose vertices may go
    /// anywhere, or that names a joint `draw` has no matrix for, which is never culled.
    pub fn placed(&self, draw: &Draw) -> Option<Aabb> {
        match (&draw.joints, self.skinned) {
            (Some(matrices), true) => {
                let boxes = self.joints.as_ref()?;
                if boxes.len() > matrices.len() {
                    return None;
                }
                Some(
                    boxes
                        .iter()
                        .zip(matrices)
                        .fold(Aabb::EMPTY, |bounds, (joint, matrix)| {
                            bounds.union(joint.transformed(*matrix))
                        }),
                )
            }
            _ => Some(self.local.transformed(draw.world)),
        }
    }
}

/// Each joint's box of the vertices it moves, by its index in the skin, as many as the last joint any vertex is
/// weighted to; `None` where the weights do not sum to one or are negative.
fn joint_bounds(primitive: &Primitive) -> Option<Vec<Aabb>> {
    let mut boxes = Vec::new();
    for ((position, influences), weights) in primitive
        .positions
        .iter()
        .zip(&primitive.joints)
        .zip(&primitive.weights)
    {
        let total: f32 = weights.iter().sum();
        if (total - 1.0).abs() > 1e-3 || weights.iter().any(|weight| *weight < 0.0) {
            return None;
        }
        for (joint, weight) in influences.iter().zip(weights) {
            if *weight > 0.0 {
                let joint = usize::from(*joint);
                if boxes.len() <= joint {
                    boxes.resize(joint + 1, Aabb::EMPTY);
                }
                boxes[joint] = boxes[joint].with(*position);
            }
        }
    }
    Some(boxes)
}

/// Whether `draw` turns its primitives inside out, as a mirror does, so that what faces the eye is wound the other way
/// about: its node's matrix, or any joint's that moves it, reverses handedness.
pub fn mirrored(draw: &Draw) -> bool {
    match &draw.joints {
        Some(matrices) => matrices.iter().any(|matrix| matrix.determinant() < 0.0),
        None => draw.world.determinant() < 0.0,
    }
}

/// Whether `material` of `model`, if any, hides what is behind it everywhere it is drawn: neither cut out nor, by its
/// texture, given holes.
pub fn opaque(model: &Model, material: Option<usize>) -> bool {
    material
        .map(|material| &model.materials[material])
        .is_none_or(|material| !material.masked)
}

impl Primitive {
    /// Whether its triangles close every part of it, each wound counterclockwise seen from outside, as
    /// [`Primitive::closed_triangles`] says of each triangle.
    pub fn closed(&self) -> bool {
        let closed = self.closed_triangles();
        !closed.is_empty() && closed.iter().all(|closed| *closed)
    }

    /// For each of its triangles, whether the part of it the triangle belongs to, of triangles joined at their edges
    /// where their vertices are, is closed and faces out: every edge in it shared by exactly two of its triangles,
    /// crossing it in opposite directions, so that it is wound consistently, counterclockwise seen from outside, enclosing
    /// its volume. From outside such a part, whatever of it faces away from the eye is behind what of it faces the eye,
    /// so its back faces need not be drawn. A part with an edge three triangles share, or two crossing it the same way,
    /// with an edge only one triangle has, as an open box's or a card's, or with next to no volume, as a card drawn on
    /// both sides as two layers, is not closed, nor is a triangle with two corners at one point.
    pub fn closed_triangles(&self) -> Vec<bool> {
        // Vertices where the same point is, as a faceted mesh gives each face its own, are one.
        let mut points: HashMap<[u32; 3], u32> = HashMap::new();
        let mut welded = |index: u32| {
            let position = self.positions[index as usize];
            let next = points.len() as u32;
            *points
                .entry(position.to_array().map(f32::to_bits))
                .or_insert(next)
        };
        let triangles: Vec<[u32; 3]> = self
            .indices
            .as_chunks::<3>()
            .0
            .iter()
            .map(|corners| corners.map(&mut welded))
            .collect();
        // Each directed edge, with the triangles that cross it so.
        let mut edges: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
        let mut open = vec![false; triangles.len()];
        for (triangle, [a, b, c]) in triangles.iter().enumerate() {
            if a == b || b == c || c == a {
                open[triangle] = true;
                continue;
            }
            for edge in [(*a, *b), (*b, *c), (*c, *a)] {
                edges.entry(edge).or_default().push(triangle);
            }
        }
        // The parts, of triangles joined at their edges either way.
        let mut part: Vec<usize> = (0..triangles.len()).collect();
        fn root(part: &mut [usize], mut at: usize) -> usize {
            while part[at] != at {
                part[at] = part[part[at]];
                at = part[at];
            }
            at
        }
        for (&(a, b), crossing) in &edges {
            let reverse = edges.get(&(b, a));
            let joined = crossing.iter().chain(reverse.into_iter().flatten());
            let first = crossing[0];
            for &other in joined {
                let (one, two) = (root(&mut part, first), root(&mut part, other));
                part[one] = two;
            }
            // An edge crossed the same way twice, or never the other way, opens its part.
            if crossing.len() != 1 || reverse.is_none_or(|reverse| reverse.len() != 1) {
                open[first] = true;
            }
        }
        // Each part's volume, and the box about it, measured about its own middle, so that a small part far from the
        // model's origin loses nothing to rounding.
        let corners = |triangle: usize| {
            [0, 1, 2].map(|index| self.positions[self.indices[triangle * 3 + index] as usize])
        };
        let mut parts: HashMap<usize, (bool, f32, Aabb)> = HashMap::new();
        for (triangle, open) in open.iter().enumerate() {
            let (opened, _, bounds) =
                parts
                    .entry(root(&mut part, triangle))
                    .or_insert((false, 0.0, Aabb::EMPTY));
            *opened |= *open;
            *bounds = corners(triangle).into_iter().fold(*bounds, Aabb::with);
        }
        for triangle in 0..triangles.len() {
            let (_, volume, bounds) = parts
                .get_mut(&root(&mut part, triangle))
                .expect("every triangle's part is measured");
            let middle = (bounds.min + bounds.max) * 0.5;
            let [a, b, c] = corners(triangle).map(|corner| corner - middle);
            *volume += a.dot(b.cross(c)) / 6.0;
        }
        (0..triangles.len())
            .map(|triangle| {
                let (opened, volume, bounds) = parts[&root(&mut part, triangle)];
                let across = (bounds.max - bounds.min).length();
                !opened && volume > 1e-6 * across * across * across
            })
            .collect()
    }
}

#[cfg(test)]
pub(crate) mod tests;
