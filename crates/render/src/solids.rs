// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Volumes' solids: a handful of unit meshes, a box, a sphere, a cylinder, a hexagonal prism, a diamond, a cone and a
//! tube, each drawn in one instanced draw for every volume in view, by the pipeline the swarm and the workers are drawn
//! with, opaque and writing depth, lit from above. A frame's instances are written once, within a budget of triangles,
//! beyond which the volumes last in the frame's order are drawn without their solids. See `design/diagrams.md`.
use std::ops::Range;

use tomori_scene::projection::volume::{Drawn, Solid};
use wgpu::util::DeviceExt;

use crate::graphics::{Instance, Vertex};

/// The most solids drawn in a frame, and the most triangles they take together: about what sixteen volumes of 64
/// nodes, at the headset's limits, take.
pub const MOST_SOLIDS: usize = 16_384;
pub const TRIANGLES: usize = 100_000;

/// A unit sphere's segments about its axis and its rings from pole to pole: 80 triangles.
const SEGMENTS: usize = 8;
const RINGS: usize = 6;

/// Each solid's unit mesh, within -0.5 to 0.5 on each axis, as triangles of vertices with their normals.
pub(crate) fn mesh(solid: Solid) -> Vec<Vertex> {
    match solid {
        Solid::Box => crate::graphics::cube(),
        Solid::Sphere => sphere(),
        Solid::Cylinder => prism(12, true),
        Solid::Prism => prism(6, true),
        Solid::Tube => prism(6, false),
        Solid::Diamond => diamond(),
        Solid::Cone => cone(8),
    }
}

fn vertex(position: glam::Vec3, normal: glam::Vec3) -> Vertex {
    Vertex {
        position: position.to_array(),
        normal: normal.normalize_or_zero().to_array(),
    }
}

/// A sphere of [`SEGMENTS`] by [`RINGS`], of radius a half.
fn sphere() -> Vec<Vertex> {
    let at = |ring: usize, segment: usize| {
        let polar = std::f32::consts::PI * ring as f32 / RINGS as f32;
        let around = std::f32::consts::TAU * segment as f32 / SEGMENTS as f32;
        glam::Vec3::new(
            polar.sin() * around.cos(),
            polar.cos(),
            polar.sin() * around.sin(),
        )
    };
    let mut vertices = Vec::new();
    for ring in 0..RINGS {
        for segment in 0..SEGMENTS {
            let corners = [
                at(ring, segment),
                at(ring + 1, segment),
                at(ring + 1, segment + 1),
                at(ring, segment + 1),
            ];
            let triangles: &[[usize; 3]] = if ring == 0 {
                &[[0, 2, 1]]
            } else if ring == RINGS - 1 {
                &[[0, 3, 1]]
            } else {
                &[[0, 2, 1], [0, 3, 2]]
            };
            for triangle in triangles {
                for corner in triangle {
                    vertices.push(vertex(corners[*corner] * 0.5, corners[*corner]));
                }
            }
        }
    }
    vertices
}

/// A prism of `sides` about its height, the Y axis, with its ends closed where `capped`.
fn prism(sides: usize, capped: bool) -> Vec<Vertex> {
    let at = |side: usize| {
        let angle = std::f32::consts::TAU * side as f32 / sides as f32;
        glam::Vec3::new(angle.cos() * 0.5, 0.0, angle.sin() * 0.5)
    };
    let (up, down) = (glam::Vec3::Y * 0.5, -glam::Vec3::Y * 0.5);
    let mut vertices = Vec::new();
    for side in 0..sides {
        let (a, b) = (at(side), at(side + 1));
        let normal = (a + b) / 2.0;
        for corner in [a + down, b + up, b + down, a + down, a + up, b + up] {
            vertices.push(vertex(corner, normal));
        }
        if capped {
            for corner in [up, b + up, a + up] {
                vertices.push(vertex(corner, glam::Vec3::Y));
            }
            for corner in [down, a + down, b + down] {
                vertices.push(vertex(corner, -glam::Vec3::Y));
            }
        }
    }
    vertices
}

/// A cone of `sides`, its point up the Y axis and its base closed.
fn cone(sides: usize) -> Vec<Vertex> {
    let at = |side: usize| {
        let angle = std::f32::consts::TAU * side as f32 / sides as f32;
        glam::Vec3::new(angle.cos() * 0.5, -0.5, angle.sin() * 0.5)
    };
    let (tip, base) = (glam::Vec3::Y * 0.5, -glam::Vec3::Y * 0.5);
    let mut vertices = Vec::new();
    for side in 0..sides {
        let (a, b) = (at(side), at(side + 1));
        let normal = ((a + b) / 2.0 - base) + glam::Vec3::Y * 0.5;
        for corner in [a, tip, b] {
            vertices.push(vertex(corner, normal));
        }
        for corner in [base, a, b] {
            vertices.push(vertex(corner, -glam::Vec3::Y));
        }
    }
    vertices
}

/// An octahedron, its points a half out along each axis.
fn diamond() -> Vec<Vertex> {
    let mut vertices = Vec::new();
    for x in [-0.5f32, 0.5] {
        for y in [-0.5f32, 0.5] {
            for z in [-0.5f32, 0.5] {
                let normal = glam::Vec3::new(x, y, z);
                let corners = [glam::Vec3::X * x, glam::Vec3::Y * y, glam::Vec3::Z * z];
                // Wound the same way seen from outside, whichever octant.
                let order = if x * y * z > 0.0 {
                    [0, 1, 2]
                } else {
                    [0, 2, 1]
                };
                for index in order {
                    vertices.push(vertex(corners[index], normal));
                }
            }
        }
    }
    vertices
}

/// Volumes' solids on the GPU: every unit mesh in one buffer, and a frame's instances, by solid, in another.
pub struct Solids {
    vertices: wgpu::Buffer,
    instances: wgpu::Buffer,
    /// Each solid's vertices in the mesh buffer, in [`Solid::ALL`]'s order.
    meshes: Vec<Range<u32>>,
    /// The frame's instances, by solid, in the instance buffer.
    drawn: Vec<(usize, Range<u32>)>,
    /// The frame's instances, gathered.
    staged: Vec<tomori_scene::projection::volume::Instance>,
    /// How many triangles the frame draws, and how many volumes it drew without their solids, for its log and tests.
    triangles: usize,
    left_out: usize,
}

impl Solids {
    pub fn new(device: &wgpu::Device) -> Self {
        let mut all = Vec::new();
        let mut meshes = Vec::new();
        for solid in Solid::ALL {
            let start = all.len() as u32;
            all.extend(mesh(solid));
            meshes.push(start..all.len() as u32);
        }
        let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("volume solids"),
            contents: bytemuck::cast_slice(&all),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let instances = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("volume instances"),
            size: (MOST_SOLIDS * size_of::<Instance>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            vertices,
            instances,
            meshes,
            drawn: Vec::new(),
            staged: Vec::new(),
            triangles: 0,
            left_out: 0,
        }
    }

    /// Gather the solids of the frame's `volumes`, in order, within [`MOST_SOLIDS`] and [`TRIANGLES`], and write them.
    pub fn update(&mut self, queue: &wgpu::Queue, volumes: &[Drawn]) {
        let triangles = |solid: Solid| self.meshes[index(solid)].len() / 3;
        let mut kept: Vec<tomori_scene::projection::volume::Instance> = Vec::new();
        let (mut total, mut left_out) = (0, 0);
        for volume in volumes {
            self.staged.clear();
            volume.instances(&mut self.staged);
            let cost: usize = self
                .staged
                .iter()
                .map(|instance| triangles(instance.solid))
                .sum();
            if total + cost > TRIANGLES || kept.len() + self.staged.len() > MOST_SOLIDS {
                left_out += 1;
                continue;
            }
            total += cost;
            kept.extend(self.staged.iter().copied());
        }
        kept.sort_by_key(|instance| index(instance.solid));
        let instances: Vec<Instance> = kept
            .iter()
            .map(|instance| Instance {
                position: instance.position.to_array(),
                scale: instance.scale.to_array(),
                color: instance.colour,
                basis: instance.basis.to_cols_array_2d(),
            })
            .collect();
        self.drawn.clear();
        for (at, instance) in kept.iter().enumerate() {
            let solid = index(instance.solid);
            match self.drawn.last_mut() {
                Some((last, range)) if *last == solid => range.end = at as u32 + 1,
                _ => self.drawn.push((solid, at as u32..at as u32 + 1)),
            }
        }
        if !instances.is_empty() {
            queue.write_buffer(&self.instances, 0, bytemuck::cast_slice(&instances));
        }
        self.triangles = total;
        self.left_out = left_out;
    }

    /// How many triangles the frame's solids take, and how many volumes it drew without them.
    pub fn counts(&self) -> (usize, usize) {
        (self.triangles, self.left_out)
    }

    /// Draw the frame's solids with `pipeline`, the swarm's, whose camera is bound.
    pub(crate) fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        if self.drawn.is_empty() {
            return;
        }
        pass.set_vertex_buffer(0, self.vertices.slice(..));
        pass.set_vertex_buffer(1, self.instances.slice(..));
        for (solid, range) in &self.drawn {
            pass.draw(self.meshes[*solid].clone(), range.clone());
        }
    }
}

/// `solid`'s place in [`Solid::ALL`].
fn index(solid: Solid) -> usize {
    Solid::ALL
        .iter()
        .position(|known| *known == solid)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests;
