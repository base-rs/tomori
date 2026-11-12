// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! A bounding volume hierarchy over a root's static instances, built with binned surface area heuristics into a flat
//! array when they change, and walked once a frame against the views' frustum. It finds exactly what testing each
//! instance's box against the frustum finds: a node wholly outside one of the frustum's planes holds nothing seen, and
//! one wholly inside a plane holds nothing that plane culls, as each box within it is within it.
use glam::{Vec3, Vec4, Vec4Swizzles};

use crate::{Aabb, Frustum};

/// One instance in the hierarchy: its box, finite and not empty, and the item's slot and the instance it is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Leaf {
    pub bounds: Aabb,
    pub slot: u32,
    pub instance: u32,
}

/// A node: the box about its leaves, `leaves[first..first + count]`, contiguous; and, if it is split, its second
/// child, its first being the node after it, or 0 for a node that is not.
#[derive(Clone, Copy, Debug)]
struct Node {
    bounds: Aabb,
    first: u32,
    count: u32,
    second: u32,
}

/// How many leaves a node holds at most before it is split, unless it cannot be.
const LEAF: usize = 4;
/// How many bins each axis is cut into to choose a split.
const BINS: usize = 8;
/// How deep a split chosen by its surface area goes before splitting at the median, which keeps the depth within about
/// this and the logarithm of the leaves, whatever the boxes.
const DEPTH: usize = 32;
/// Every one of a frustum's planes.
const PLANES: u8 = 0b11_1111;

#[derive(Debug, Default)]
pub(crate) struct Bvh {
    nodes: Vec<Node>,
    leaves: Vec<Leaf>,
    /// The nodes left to visit as it is walked, with the planes each is not yet wholly inside, kept between walks, as
    /// many as it is deep, so that walking it never allocates.
    stack: Vec<(u32, u8)>,
    /// How deep it is, as built.
    deepest: usize,
}

/// Half the surface of a box, which is all a split's cost needs, or none for an empty box.
fn area(bounds: &Aabb) -> f32 {
    if bounds.is_empty() {
        return 0.0;
    }
    let size = bounds.max - bounds.min;
    size.x * size.y + size.y * size.z + size.z * size.x
}

fn centre(bounds: &Aabb) -> Vec3 {
    (bounds.min + bounds.max) * 0.5
}

/// How far `point` is inside `plane`, as [`Frustum::sees`] measures it, so that the two agree exactly.
fn inside(plane: Vec4, point: Vec3) -> f32 {
    plane.xyz().dot(point) + plane.w
}

impl Bvh {
    /// The hierarchy over `leaves`, in place of what it held, keeping its storage.
    pub fn build(&mut self, leaves: impl IntoIterator<Item = Leaf>) {
        self.build_within(leaves, DEPTH);
    }

    /// [`Bvh::build`], splitting by surface area no deeper than `depth`.
    fn build_within(&mut self, leaves: impl IntoIterator<Item = Leaf>, depth: usize) {
        self.leaves.clear();
        self.leaves.extend(leaves);
        self.nodes.clear();
        self.deepest = 0;
        if !self.leaves.is_empty() {
            self.node(0, self.leaves.len(), depth, 1);
        }
        // Walking it first, then second, keeps one sibling of each node above for each level it goes down.
        self.stack.clear();
        self.stack.reserve(self.deepest + 1);
    }

    /// How many leaves it holds.
    pub fn len(&self) -> usize {
        self.leaves.len()
    }

    /// Make the node over `leaves[first..first + count]`, and its children, returning its index; it is `level` deep.
    fn node(&mut self, first: usize, count: usize, depth: usize, level: usize) -> u32 {
        self.deepest = self.deepest.max(level);
        let leaves = &self.leaves[first..first + count];
        let bounds = leaves
            .iter()
            .fold(Aabb::EMPTY, |bounds, leaf| bounds.union(leaf.bounds));
        let index = self.nodes.len() as u32;
        self.nodes.push(Node {
            bounds,
            first: first as u32,
            count: count as u32,
            second: 0,
        });
        if count > LEAF {
            let middle = self
                .split(first, count, depth)
                .unwrap_or_else(|| self.median(first, count));
            let depth = depth.saturating_sub(1);
            self.node(first, middle - first, depth, level + 1);
            let second = self.node(middle, first + count - middle, depth, level + 1);
            self.nodes[index as usize].second = second;
        }
        index
    }

    /// Split `leaves[first..first + count]` where its surface area is least, on whichever axis, by its leaves' centres
    /// in bins, moving them to either side, and say where the second side begins; or none, when `depth` is spent or no
    /// bin divides them.
    fn split(&mut self, first: usize, count: usize, depth: usize) -> Option<usize> {
        if depth == 0 {
            return None;
        }
        let leaves = &mut self.leaves[first..first + count];
        let centres = leaves.iter().fold(Aabb::EMPTY, |bounds, leaf| {
            bounds.with(centre(&leaf.bounds))
        });
        let extent = centres.max - centres.min;
        let bin = |axis: usize, leaf: &Leaf| {
            let at = (centre(&leaf.bounds)[axis] - centres.min[axis]) / extent[axis];
            ((at * BINS as f32) as usize).min(BINS - 1)
        };
        let mut best: Option<(f32, usize, usize)> = None;
        for axis in 0..3 {
            if extent[axis] <= f32::EPSILON * centres.max[axis].abs().max(1.0) {
                continue;
            }
            let mut bins = [(Aabb::EMPTY, 0usize); BINS];
            for leaf in leaves.iter() {
                let (bounds, count) = &mut bins[bin(axis, leaf)];
                *bounds = bounds.union(leaf.bounds);
                *count += 1;
            }
            // The cost of each cut between bins: the area on each side by how many leaves it holds. Neither side is ever
            // empty, as the least centre is in the first bin and the greatest in the last.
            let mut below = [(0.0f32, 0usize); BINS];
            let (mut bounds, mut held) = (Aabb::EMPTY, 0);
            for cut in 0..BINS - 1 {
                bounds = bounds.union(bins[cut].0);
                held += bins[cut].1;
                below[cut] = (area(&bounds), held);
            }
            let (mut bounds, mut held) = (Aabb::EMPTY, 0);
            for cut in (0..BINS - 1).rev() {
                bounds = bounds.union(bins[cut + 1].0);
                held += bins[cut + 1].1;
                let (area_below, held_below) = below[cut];
                let cost = area_below * held_below as f32 + area(&bounds) * held as f32;
                if best.is_none_or(|(least, ..)| cost < least) {
                    best = Some((cost, axis, cut));
                }
            }
        }
        let (_, axis, cut) = best?;
        // Those in the cut's bins and below first, then the rest.
        let mut second = 0;
        for at in 0..leaves.len() {
            if bin(axis, &leaves[at]) <= cut {
                leaves.swap(at, second);
                second += 1;
            }
        }
        Some(first + second)
    }

    /// Split `leaves[first..first + count]` in half by their centres along the axis they spread most along, and say
    /// where the second half begins.
    fn median(&mut self, first: usize, count: usize) -> usize {
        let leaves = &mut self.leaves[first..first + count];
        let centres = leaves.iter().fold(Aabb::EMPTY, |bounds, leaf| {
            bounds.with(centre(&leaf.bounds))
        });
        let axis = (centres.max - centres.min).max_position();
        leaves.select_nth_unstable_by(count / 2, |a, b| {
            centre(&a.bounds)[axis].total_cmp(&centre(&b.bounds)[axis])
        });
        first + count / 2
    }

    /// Call `seen` with each leaf `frustum` may see, as [`Frustum::sees`] would say of its box.
    pub fn walk(&mut self, frustum: &Frustum, mut seen: impl FnMut(&Leaf)) {
        if self.nodes.is_empty() {
            return;
        }
        let planes = frustum.planes();
        self.stack.clear();
        self.stack.push((0, PLANES));
        while let Some((index, mut testing)) = self.stack.pop() {
            let node = self.nodes[index as usize];
            let mut outside = false;
            for (side, plane) in planes.iter().enumerate() {
                if testing & (1 << side) == 0 {
                    continue;
                }
                let toward = plane.xyz().cmpge(Vec3::ZERO);
                // The corner farthest along the plane's normal, and the nearest.
                if inside(
                    *plane,
                    Vec3::select(toward, node.bounds.max, node.bounds.min),
                ) < 0.0
                {
                    outside = true;
                    break;
                }
                if inside(
                    *plane,
                    Vec3::select(toward, node.bounds.min, node.bounds.max),
                ) >= 0.0
                {
                    testing &= !(1 << side);
                }
            }
            if outside {
                continue;
            }
            let leaves = &self.leaves[node.first as usize..(node.first + node.count) as usize];
            if testing == 0 {
                // Wholly inside every plane: all of it is seen.
                leaves.iter().for_each(&mut seen);
            } else if node.second == 0 {
                for leaf in leaves {
                    let within = planes.iter().enumerate().all(|(side, plane)| {
                        testing & (1 << side) == 0
                            || inside(
                                *plane,
                                Vec3::select(
                                    plane.xyz().cmpge(Vec3::ZERO),
                                    leaf.bounds.max,
                                    leaf.bounds.min,
                                ),
                            ) >= 0.0
                    });
                    if within {
                        seen(leaf);
                    }
                }
            } else {
                self.stack.push((node.second, testing));
                self.stack.push((index + 1, testing));
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests;
