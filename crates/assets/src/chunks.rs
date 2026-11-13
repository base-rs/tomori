// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Large static primitives split into chunks across the ground, so that a render world culls and lights each chunk on
//! its own rather than the whole: a scenery's ground tens of metres across is otherwise always in view, and reached by
//! more lamps than light one instance. Each chunk is a primitive of its own, of the same material, its triangles the
//! very ones it had, every corner's attributes copied as they were, so that the chunks together draw what it drew, with
//! no seam where they meet. See `design/rendering.md`.
use std::collections::HashMap;

use glam::Vec3;

use crate::bounds::Aabb;
use crate::model::{Model, Primitive};

impl Primitive {
    /// Its triangles in chunks across the ground, along a grid of its box in x and z cut evenly into cells at most
    /// `size` across, each chunk a primitive of the triangles of one cell, in their order, with their vertices'
    /// attributes as they were; or itself whole, if it fits in one cell, is skinned, or `size` is not a positive number.
    /// A part of it, of triangles joined at their edges as [`Primitive::closed_triangles`] finds them, no wider across
    /// the ground than half of `size` is kept whole, in the cell its box's middle is in, so that a post or a stone is
    /// never cut; a wider one that is closed, as a long rail is, is a chunk of its own, so that it is still drawn without
    /// its back faces and stretches no cell; and a wider one that is open, as a ground is, is cut by where each
    /// triangle's middle is. No triangle is cut, so a chunk reaches beyond its cell by as much as its triangles and parts
    /// there do, and a triangle longer than a cell, as a fan of slivers has, makes its chunk as long.
    pub fn chunks(&self, size: f32) -> Vec<Primitive> {
        let (low, high) = self.bounds();
        let extent = high - low;
        let across = |extent: f32| (extent / size).ceil().max(1.0) as usize;
        if !(size > 0.0 && size.is_finite()) || !self.joints.is_empty() {
            return vec![self.clone()];
        }
        let cells = (across(extent.x), across(extent.z));
        if cells == (1, 1) {
            return vec![self.clone()];
        }
        let corners = |triangle: usize| {
            [0, 1, 2].map(|corner| self.positions[self.indices[triangle * 3 + corner] as usize])
        };
        // Each part's box, by the first of its triangles.
        let parts = self.parts();
        let mut boxes: HashMap<usize, Aabb> = HashMap::new();
        for (triangle, (part, _)) in parts.iter().enumerate() {
            let bounds = boxes.entry(*part).or_insert(Aabb::EMPTY);
            *bounds = corners(triangle).into_iter().fold(*bounds, Aabb::with);
        }
        let cell = |point: Vec3| {
            // Along an axis of one cell, which may be flat, every point is in it.
            let along = |at: f32, low: f32, extent: f32, cells: usize| {
                if cells == 1 {
                    return 0;
                }
                ((at - low) / extent * cells as f32)
                    .floor()
                    .clamp(0.0, cells as f32 - 1.0) as usize
            };
            along(point.z, low.z, extent.z, cells.1) * cells.0
                + along(point.x, low.x, extent.x, cells.0)
        };
        // Each cell's triangles, then each closed part wider than half a cell, a chunk of its own, by its first triangle.
        let mut chunks: Vec<Vec<usize>> = vec![Vec::new(); cells.0 * cells.1];
        let mut alone: Vec<(usize, Vec<usize>)> = Vec::new();
        for (triangle, (part, closed)) in parts.iter().enumerate() {
            let bounds = boxes[part];
            let wide = (bounds.max - bounds.min).x.max((bounds.max - bounds.min).z) > size * 0.5;
            if *closed && wide {
                match alone.iter_mut().find(|(first, _)| first == part) {
                    Some((_, triangles)) => triangles.push(triangle),
                    None => alone.push((*part, vec![triangle])),
                }
                continue;
            }
            let middle = if wide {
                corners(triangle).into_iter().sum::<Vec3>() / 3.0
            } else {
                (bounds.min + bounds.max) * 0.5
            };
            chunks[cell(middle)].push(triangle);
        }
        chunks
            .into_iter()
            .chain(alone.into_iter().map(|(_, triangles)| triangles))
            .filter(|triangles| !triangles.is_empty())
            .map(|triangles| self.of(&triangles))
            .collect()
    }

    /// A primitive of `triangles` of it alone, in their order, each vertex they use once, in the order they first use
    /// it, with every attribute it has.
    fn of(&self, triangles: &[usize]) -> Primitive {
        let mut index: HashMap<u32, u32> = HashMap::new();
        let mut used = Vec::new();
        let indices = triangles
            .iter()
            .flat_map(|triangle| &self.indices[triangle * 3..triangle * 3 + 3])
            .map(|vertex| {
                *index.entry(*vertex).or_insert_with(|| {
                    used.push(*vertex as usize);
                    used.len() as u32 - 1
                })
            })
            .collect();
        fn picked<T: Copy>(values: &[T], used: &[usize]) -> Vec<T> {
            if values.is_empty() {
                Vec::new()
            } else {
                used.iter().map(|vertex| values[*vertex]).collect()
            }
        }
        Primitive {
            positions: picked(&self.positions, &used),
            normals: picked(&self.normals, &used),
            uvs: picked(&self.uvs, &used),
            colors: picked(&self.colors, &used),
            joints: picked(&self.joints, &used),
            weights: picked(&self.weights, &used),
            indices,
            material: self.material,
        }
    }
}

impl Model {
    /// This model with each primitive of its meshes split into chunks at most `size` across the ground, as
    /// [`Primitive::chunks`] splits it, in their place, one after another; a mesh a skinned node carries is kept whole,
    /// as its bounds are its joints'. Built once, as a scenery is made, so that each chunk is a piece of its own to cull
    /// and to light; its nodes, materials and the rest are as they were.
    pub fn chunked(&self, size: f32) -> Model {
        let mut model = self.clone();
        for (index, mesh) in model.meshes.iter_mut().enumerate() {
            let skinned = self
                .nodes
                .iter()
                .any(|node| node.mesh == Some(index) && node.skin.is_some());
            if !skinned {
                mesh.primitives = mesh
                    .primitives
                    .iter()
                    .flat_map(|primitive| primitive.chunks(size))
                    .collect();
            }
        }
        model
    }
}

#[cfg(test)]
mod tests;
