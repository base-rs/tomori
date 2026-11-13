// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Coarser primitives for drawing far away: a primitive's triangles simplified, built once as a scenery is made, by
//! collapsing edges whose loss strays its surface least, each judged by the planes of the triangles it removes and
//! moves, as Garland and Heckbert's quadrics measure it, until any further collapse would stray it further than asked.
//! Each collapse moves one vertex onto a neighbour, so that every vertex kept is where it was, to the bit; the edges of
//! an open surface, as a chunk's are where it was cut from its neighbours, never move, so that the chunks of a ground
//! still meet without a seam; and where attributes change across an edge, as between two faces of another colour, the
//! edge is kept or collapsed only along itself, so that colours stay where they were. See `design/rendering.md`.
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

use glam::DVec3;

use crate::model::{Model, Primitive};

/// A quadric of planes: the sum, over planes `n·p + d = 0` with `n` of unit length, of the squared distance from them,
/// as the upper triangle of its symmetric 4 × 4 matrix.
#[derive(Clone, Copy, Debug, Default)]
struct Quadric([f64; 10]);

impl Quadric {
    /// The plane through `point` facing `normal`, of unit length, or nothing if it has none.
    fn plane(normal: DVec3, point: DVec3) -> Self {
        let Some(n) = normal.try_normalize() else {
            return Self::default();
        };
        let d = -n.dot(point);
        Self([
            n.x * n.x,
            n.x * n.y,
            n.x * n.z,
            n.x * d,
            n.y * n.y,
            n.y * n.z,
            n.y * d,
            n.z * n.z,
            n.z * d,
            d * d,
        ])
    }

    /// These planes, each counted `weight` times.
    fn scaled(self, weight: f64) -> Self {
        Self(self.0.map(|value| value * weight))
    }

    fn add(&mut self, other: &Self) {
        for (a, b) in self.0.iter_mut().zip(other.0) {
            *a += b;
        }
    }

    /// The sum of squared distances of `p` from its planes.
    fn error(&self, p: DVec3) -> f64 {
        let [xx, xy, xz, xw, yy, yz, yw, zz, zw, ww] = self.0;
        (xx * p.x * p.x + yy * p.y * p.y + zz * p.z * p.z)
            + 2.0 * (xy * p.x * p.y + xz * p.x * p.z + yz * p.y * p.z)
            + 2.0 * (xw * p.x + yw * p.y + zw * p.z)
            + ww
    }
}

/// Where a vertex may go: anywhere among its neighbours, inside a surface of one set of attributes; only along the
/// seam it is on, between two; or nowhere, on an open edge, where more than two triangles meet, or where seams meet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    Interior,
    Seam,
    Locked,
}

/// A collapse to try: of `from` onto `to`, straying the surface by `cost`, as their versions were when it was costed.
#[derive(Debug, PartialEq)]
struct Candidate {
    cost: f64,
    from: u32,
    to: u32,
    versions: (u32, u32),
}

impl Eq for Candidate {}

impl Ord for Candidate {
    /// The least cost first, as a [`BinaryHeap`] pops the greatest.
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .cost
            .total_cmp(&self.cost)
            .then_with(|| (other.from, other.to).cmp(&(self.from, self.to)))
    }
}

impl PartialOrd for Candidate {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// A primitive's surface as it is simplified: its points, each where vertices of one position are, and its triangles,
/// each of three points and, for each corner, the vertex whose normal and colour it takes, and the vertex whose place
/// in a texture and joints it takes, which are one but on a faceted surface.
struct Surface {
    points: Vec<DVec3>,
    triangles: Vec<[u32; 3]>,
    corners: Vec<[u32; 3]>,
    textures: Vec<[u32; 3]>,
    alive: Vec<bool>,
    /// The triangles about each point, some no longer alive or about it, which are skipped.
    about: Vec<Vec<u32>>,
    quadrics: Vec<Quadric>,
    classes: Vec<Class>,
    /// Whether each corner keeps its own normal and colour wherever it goes, as a faceted surface's do.
    faceted: bool,
    /// The seam edges, both ways, by their points, with how firmly each holds.
    seams: HashMap<(u32, u32), f64>,
    /// Each vertex's attributes, as one number for vertices whose attributes are the same: on a faceted surface, those
    /// but its normal and colour.
    attributes: Vec<u32>,
    versions: Vec<u32>,
    removed: Vec<bool>,
}

impl Surface {
    fn new(primitive: &Primitive) -> Self {
        let mut welded: HashMap<[u32; 3], u32> = HashMap::new();
        let mut points = Vec::new();
        let point_of: Vec<u32> = primitive
            .positions
            .iter()
            .map(|position| {
                *welded
                    .entry(position.to_array().map(f32::to_bits))
                    .or_insert_with(|| {
                        points.push(position.as_dvec3());
                        points.len() as u32 - 1
                    })
            })
            .collect();
        let mut triangles = Vec::new();
        let mut corners = Vec::new();
        for corner in primitive.indices.as_chunks::<3>().0 {
            let triangle = corner.map(|vertex| point_of[vertex as usize]);
            let [a, b, c] = triangle;
            // A triangle with two corners at one point draws nothing, and is left out.
            if a != b && b != c && c != a {
                triangles.push(triangle);
                corners.push(*corner);
            }
        }
        // Faceted, each triangle's corners of one normal and colour, as a low-poly model's faces are, a corner keeps its
        // face's normal and colour wherever it goes, and a seam is where the texture or the colour changes across an edge,
        // not its normal, held only by the planes across it, as firmly as the colour changes, so that where faces of other
        // colours meet moves no further than any surface does, and where faces varied a little meet, a little further.
        let faceted = corners.iter().all(|corner| {
            let corner = corner.map(|vertex| vertex as usize);
            constant(&primitive.normals, corner) && constant(&primitive.colors, corner)
        });
        let attributes = attribute_classes(primitive, faceted);
        let mut about = vec![Vec::new(); points.len()];
        for (index, triangle) in triangles.iter().enumerate() {
            for point in triangle {
                about[*point as usize].push(index as u32);
            }
        }
        let firmness = |a: u32, b: u32| {
            if attributes[a as usize] != attributes[b as usize] {
                return 1.0;
            }
            match (
                primitive.colors.get(a as usize),
                primitive.colors.get(b as usize),
            ) {
                (Some(a), Some(b)) if faceted => contrast(*a, *b),
                _ => 0.0,
            }
        };
        // Each edge: how many triangles cross it each way, the vertices at its ends in the first, and how firmly it holds
        // as a seam for how its attributes change across it, not at all where they do not.
        #[derive(Default)]
        struct Edge {
            forward: u32,
            backward: u32,
            ends: Option<(u32, u32)>,
            seam: f64,
        }
        let mut edges: HashMap<(u32, u32), Edge> = HashMap::new();
        for (triangle, corner) in triangles.iter().zip(&corners) {
            for side in 0..3 {
                let (a, b) = (triangle[side], triangle[(side + 1) % 3]);
                let (va, vb) = (corner[side], corner[(side + 1) % 3]);
                let ends = if a < b { (va, vb) } else { (vb, va) };
                let edge = edges.entry((a.min(b), a.max(b))).or_default();
                if a < b {
                    edge.forward += 1;
                } else {
                    edge.backward += 1;
                }
                match edge.ends {
                    Some(seen) => {
                        edge.seam = edge
                            .seam
                            .max(firmness(seen.0, ends.0))
                            .max(firmness(seen.1, ends.1));
                    }
                    None => edge.ends = Some(ends),
                }
            }
        }
        let mut classes = vec![Class::Interior; points.len()];
        let mut seam_edges = vec![0u32; points.len()];
        let mut seams = HashMap::new();
        for ((a, b), edge) in &edges {
            if edge.forward != 1 || edge.backward != 1 {
                // Open, or met by more than two triangles, or by two wound alike.
                classes[*a as usize] = Class::Locked;
                classes[*b as usize] = Class::Locked;
            } else if edge.seam > 0.0 {
                seam_edges[*a as usize] += 1;
                seam_edges[*b as usize] += 1;
                seams.insert((*a, *b), edge.seam);
                seams.insert((*b, *a), edge.seam);
            }
        }
        for (class, count) in classes.iter_mut().zip(&seam_edges) {
            if *class == Class::Interior && !faceted {
                *class = match count {
                    0 => Class::Interior,
                    2 => Class::Seam,
                    _ => Class::Locked,
                };
            }
        }
        // Each point's planes: of the triangles about it, and, on a seam, of the planes through the seam across each
        // triangle along it, as firmly as it holds, so that a seam moved aside strays as its surface would.
        let mut quadrics = vec![Quadric::default(); points.len()];
        for triangle in &triangles {
            let [a, b, c] = triangle.map(|point| points[point as usize]);
            let normal = (b - a).cross(c - a);
            let plane = Quadric::plane(normal, a);
            for point in triangle {
                quadrics[*point as usize].add(&plane);
            }
            for side in 0..3 {
                let (from, to) = (triangle[side], triangle[(side + 1) % 3]);
                if let Some(firmness) = seams.get(&(from, to)) {
                    let (p, q) = (points[from as usize], points[to as usize]);
                    let across = Quadric::plane((q - p).cross(normal), p).scaled(*firmness);
                    quadrics[from as usize].add(&across);
                    quadrics[to as usize].add(&across);
                }
            }
        }
        Self {
            versions: vec![0; points.len()],
            removed: vec![false; points.len()],
            alive: vec![true; triangles.len()],
            points,
            triangles,
            textures: corners.clone(),
            corners,
            about,
            quadrics,
            classes,
            faceted,
            seams,
            attributes,
        }
    }

    /// The triangles alive about `point`.
    fn around(&self, point: u32) -> impl Iterator<Item = u32> + '_ {
        self.about[point as usize]
            .iter()
            .copied()
            .filter(move |triangle| {
                self.alive[*triangle as usize]
                    && self.triangles[*triangle as usize].contains(&point)
            })
    }

    /// The points joined to `point` by an edge.
    fn neighbours(&self, point: u32) -> Vec<u32> {
        let mut neighbours: Vec<u32> = self
            .around(point)
            .flat_map(|triangle| self.triangles[triangle as usize])
            .filter(|other| *other != point)
            .collect();
        neighbours.sort_unstable();
        neighbours.dedup();
        neighbours
    }

    /// Whether `from` may be collapsed onto `to`, by the classes of where it is.
    fn may(&self, from: u32, to: u32) -> bool {
        match self.classes[from as usize] {
            Class::Interior => true,
            Class::Seam => self.seams.contains_key(&(from, to)),
            Class::Locked => false,
        }
    }

    fn cost(&self, from: u32, to: u32) -> f64 {
        let mut quadric = self.quadrics[from as usize];
        quadric.add(&self.quadrics[to as usize]);
        quadric.error(self.points[to as usize]).max(0.0)
    }

    fn candidate(&self, from: u32, to: u32) -> Candidate {
        Candidate {
            cost: self.cost(from, to),
            from,
            to,
            versions: (self.versions[from as usize], self.versions[to as usize]),
        }
    }

    /// Whether collapsing `from` onto `to`, joined by an edge two triangles cross, keeps the surface whole: the two share
    /// exactly the two neighbours of those triangles, so that nothing folds onto itself, and no triangle moved turns over
    /// or vanishes.
    fn keeps(&self, from: u32, to: u32) -> bool {
        let ours = self.neighbours(from);
        let theirs = self.neighbours(to);
        let common = ours.iter().filter(|point| theirs.contains(point)).count();
        if common != 2 {
            return false;
        }
        let target = self.points[to as usize];
        self.around(from)
            .filter(|triangle| !self.triangles[*triangle as usize].contains(&to))
            .all(|triangle| {
                let corners = self.triangles[triangle as usize];
                let before = corners.map(|point| self.points[point as usize]);
                let after = corners.map(|point| {
                    if point == from {
                        target
                    } else {
                        self.points[point as usize]
                    }
                });
                let normal = |[a, b, c]: [DVec3; 3]| (b - a).cross(c - a);
                let (was, now) = (normal(before), normal(after));
                // Not turned over, nor near enough flat to turn over at a pixel's rounding.
                now.length_squared() > was.length_squared() * 1e-6 && was.dot(now) > 0.0
            })
    }

    /// Collapse `from` onto `to`: the triangles along their edge go, and the rest about `from` are about `to`, each
    /// corner taking the attributes `to` has on its side of any seam, but a faceted surface's normal and colour, which
    /// are its face's, or keeping its own where `to` has none there.
    fn collapse(&mut self, from: u32, to: u32) {
        let mut sides: Vec<(u32, u32, u32)> = Vec::new();
        let triangles: Vec<u32> = self.around(from).collect();
        for triangle in &triangles {
            let index = *triangle as usize;
            if self.triangles[index].contains(&to) {
                self.alive[index] = false;
                let at = |point: u32| {
                    self.triangles[index]
                        .iter()
                        .position(|p| *p == point)
                        .unwrap()
                };
                let (from, to) = (at(from), at(to));
                sides.push((
                    self.attributes[self.textures[index][from] as usize],
                    self.textures[index][to],
                    self.corners[index][to],
                ));
            }
        }
        for triangle in triangles {
            let index = triangle as usize;
            if !self.alive[index] {
                continue;
            }
            let at = self.triangles[index]
                .iter()
                .position(|point| *point == from)
                .unwrap();
            self.triangles[index][at] = to;
            let attributes = self.attributes[self.textures[index][at] as usize];
            if let Some((_, texture, corner)) = sides.iter().find(|(side, ..)| *side == attributes)
            {
                self.textures[index][at] = *texture;
                if !self.faceted {
                    self.corners[index][at] = *corner;
                }
            }
            self.about[to as usize].push(triangle);
        }
        let quadric = self.quadrics[from as usize];
        self.quadrics[to as usize].add(&quadric);
        self.removed[from as usize] = true;
    }
}

/// How firmly where two faces' colours meet holds as a seam: by how far apart they are, the most any channel differs of
/// the brighter, over [`SIMILAR`], squared, at most 1, so that moving it aside by a distance strays as much as moving a
/// surface by that distance times their difference over [`SIMILAR`]; faces varied a little face by face, by at most 6% of
/// their colour, as a model's are, hold a little, and the same colour not at all.
fn contrast(a: glam::Vec4, b: glam::Vec4) -> f64 {
    let brighter = a.max(b);
    let apart = (a - b).abs() / brighter.max(glam::Vec4::splat(f32::MIN_POSITIVE));
    f64::from((apart.max_element() / SIMILAR).min(1.0)).powi(2)
}

/// How much two colours differ, of the brighter, for where they meet to hold as firmly as any surface: twice the 6% a
/// model's faces are varied by, and a little more.
const SIMILAR: f32 = 0.15;

/// Whether `values`, if there are any, are the same at each of `corners`.
fn constant<T: PartialEq>(values: &[T], [a, b, c]: [usize; 3]) -> bool {
    values.is_empty() || (values[a] == values[b] && values[b] == values[c])
}

/// For each vertex, a number shared by every vertex whose attributes are the same, bit for bit, but its position, and,
/// `faceted`, its normal and colour.
fn attribute_classes(primitive: &Primitive, faceted: bool) -> Vec<u32> {
    let mut classes: HashMap<Vec<u32>, u32> = HashMap::new();
    (0..primitive.positions.len())
        .map(|vertex| {
            let mut key = Vec::new();
            if let Some(normal) = primitive.normals.get(vertex).filter(|_| !faceted) {
                key.extend(normal.to_array().map(f32::to_bits));
            }
            if let Some(uv) = primitive.uvs.get(vertex) {
                key.extend(uv.to_array().map(f32::to_bits));
            }
            if let Some(color) = primitive.colors.get(vertex).filter(|_| !faceted) {
                key.extend(color.to_array().map(f32::to_bits));
            }
            if let Some(joints) = primitive.joints.get(vertex) {
                key.extend(joints.map(u32::from));
            }
            if let Some(weights) = primitive.weights.get(vertex) {
                key.extend(weights.map(f32::to_bits));
            }
            let next = classes.len() as u32;
            *classes.entry(key).or_insert(next)
        })
        .collect()
}

impl Primitive {
    /// It drawn with fewer triangles, straying from its surface by at most about `error`, in its own units, and how far
    /// it strayed, at most: its edges collapsed least first, each moving a vertex onto a neighbour, while any can be
    /// that strays it no further, judged by the squared distances of where the vertex goes from the planes of every
    /// triangle it has been part of, and, along a seam, of the planes across it, so that the distance from any of them
    /// is no more than `error`. Kept as they are: every vertex on an open edge, where its surface was cut, as a chunk's
    /// was from its neighbours, so that they still meet; and where more than two triangles meet at an edge. On a smooth
    /// surface, where seams, edges across which attributes change, meet, so that a box's corners stay, and a vertex on a
    /// seam moves only along it; on a faceted one, each triangle of one normal and colour, a corner keeps its face's
    /// normal and colour, and where its texture or colour changes is held by the planes across it, its colour as firmly
    /// as the two differ. No collapse folds the surface onto itself or turns a triangle over, so a closed part stays
    /// closed, and every vertex kept is where it was, with the attributes it had, but those of a corner moved onto
    /// another, which takes that vertex's on its side of any seam. Its triangles are in the order they were, and its
    /// material is the same.
    pub fn simplified(&self, error: f32) -> (Primitive, f32) {
        let mut surface = Surface::new(self);
        let limit = f64::from(error.max(0.0)).powi(2);
        let mut heap = BinaryHeap::new();
        for from in 0..surface.points.len() as u32 {
            for to in surface.neighbours(from) {
                if surface.may(from, to) {
                    heap.push(surface.candidate(from, to));
                }
            }
        }
        let mut strayed = 0f64;
        while let Some(candidate) = heap.pop() {
            if candidate.cost > limit {
                break;
            }
            let (from, to) = (candidate.from, candidate.to);
            if surface.removed[from as usize]
                || surface.removed[to as usize]
                || candidate.versions
                    != (
                        surface.versions[from as usize],
                        surface.versions[to as usize],
                    )
                || !surface.keeps(from, to)
            {
                continue;
            }
            surface.collapse(from, to);
            strayed = strayed.max(candidate.cost);
            // What is about `to` now costs otherwise.
            let mut changed = surface.neighbours(to);
            changed.push(to);
            for point in &changed {
                surface.versions[*point as usize] += 1;
            }
            for point in changed {
                for other in surface.neighbours(point) {
                    if surface.may(point, other) {
                        heap.push(surface.candidate(point, other));
                    }
                }
            }
        }
        (self.rebuilt(&surface), strayed.sqrt() as f32)
    }

    /// The primitive of `surface`'s triangles alive, each corner a vertex of its attributes at its point.
    fn rebuilt(&self, surface: &Surface) -> Primitive {
        let mut vertices: HashMap<[u32; 3], u32> = HashMap::new();
        let mut used: Vec<[u32; 3]> = Vec::new();
        let mut indices = Vec::new();
        for (((triangle, corners), textures), alive) in surface
            .triangles
            .iter()
            .zip(&surface.corners)
            .zip(&surface.textures)
            .zip(&surface.alive)
        {
            if !alive {
                continue;
            }
            for ((point, corner), texture) in triangle.iter().zip(corners).zip(textures) {
                let key = [*corner, *texture, *point];
                let index = *vertices.entry(key).or_insert_with(|| {
                    used.push(key);
                    used.len() as u32 - 1
                });
                indices.push(index);
            }
        }
        // Each attribute from the vertex of the corner's normal and colour, `0`, or of its place in a texture and joints.
        fn picked<T: Copy>(values: &[T], used: &[[u32; 3]], from: usize) -> Vec<T> {
            if values.is_empty() {
                Vec::new()
            } else {
                used.iter().map(|key| values[key[from] as usize]).collect()
            }
        }
        Primitive {
            positions: used
                .iter()
                .map(|[_, _, point]| surface.points[*point as usize].as_vec3())
                .collect(),
            normals: picked(&self.normals, &used, 0),
            uvs: picked(&self.uvs, &used, 1),
            colors: picked(&self.colors, &used, 0),
            joints: picked(&self.joints, &used, 1),
            weights: picked(&self.weights, &used, 1),
            indices,
            material: self.material,
        }
    }
}

impl Model {
    /// This model with each primitive [`Primitive::simplified`] to stray by at most about `error`, in its meshes' own
    /// units, and how far the most strayed; its nodes, skins, materials and the rest as they were.
    pub fn simplified(&self, error: f32) -> (Model, f32) {
        let mut model = self.clone();
        let mut strayed = 0f32;
        for mesh in &mut model.meshes {
            for primitive in &mut mesh.primitives {
                let (simplified, by) = primitive.simplified(error);
                *primitive = simplified;
                strayed = strayed.max(by);
            }
        }
        (model, strayed)
    }
}

#[cfg(test)]
mod tests;
