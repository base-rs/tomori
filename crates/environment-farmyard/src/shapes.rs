// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Low-poly shapes, built in code, as the farmyard's scenery is: each face flat, with a normal and a colour of its own, a
//! little varied face by face, so that the facets read as a hand-made model's do.
use glam::{Quat, Vec3, Vec4};
use tomori_assets::Primitive;

/// A triangle list under construction, of flat faces.
#[derive(Default)]
pub struct Shapes {
    positions: Vec<Vec3>,
    normals: Vec<Vec3>,
    colors: Vec<Vec4>,
    indices: Vec<u32>,
    /// Varies each face's colour a little, the same each time the shapes are built.
    faces: u32,
}

/// `color`, linear RGB, as a vertex colour, varied a little by `seed`, by at most `amount` of itself.
fn varied(color: Vec3, seed: u32, amount: f32) -> Vec4 {
    let mut hash = seed.wrapping_mul(0x9e37_79b9) ^ 0x85eb_ca6b;
    hash ^= hash >> 15;
    hash = hash.wrapping_mul(0x2c1b_3c6d);
    hash ^= hash >> 12;
    let unit = (hash & 0xffff) as f32 / 65535.0;
    (color * (1.0 + (unit - 0.5) * 2.0 * amount)).extend(1.0)
}

impl Shapes {
    pub fn new() -> Self {
        Self::default()
    }

    /// A flat polygon through `corners`, in order, lit from the side they wind anticlockwise towards.
    pub fn polygon(&mut self, corners: &[Vec3], color: Vec3) {
        let normal = (corners[1] - corners[0])
            .cross(corners[2] - corners[0])
            .normalize_or_zero();
        let color = varied(color, self.faces, 0.06);
        self.faces += 1;
        let first = self.positions.len() as u32;
        for corner in corners {
            self.positions.push(*corner);
            self.normals.push(normal);
            self.colors.push(color);
        }
        for index in 1..corners.len() as u32 - 1 {
            self.indices
                .extend([first, first + index, first + index + 1]);
        }
    }

    /// A box centred at `centre`, `half` its size on each axis, turned by `turn` about its centre.
    pub fn cuboid(&mut self, centre: Vec3, half: Vec3, turn: Quat, color: Vec3) {
        let corner = |x: f32, y: f32, z: f32| centre + turn * (half * Vec3::new(x, y, z));
        let faces = [
            [
                (1.0, -1.0, -1.0),
                (1.0, 1.0, -1.0),
                (1.0, 1.0, 1.0),
                (1.0, -1.0, 1.0),
            ],
            [
                (-1.0, -1.0, 1.0),
                (-1.0, 1.0, 1.0),
                (-1.0, 1.0, -1.0),
                (-1.0, -1.0, -1.0),
            ],
            [
                (-1.0, 1.0, -1.0),
                (-1.0, 1.0, 1.0),
                (1.0, 1.0, 1.0),
                (1.0, 1.0, -1.0),
            ],
            [
                (-1.0, -1.0, 1.0),
                (-1.0, -1.0, -1.0),
                (1.0, -1.0, -1.0),
                (1.0, -1.0, 1.0),
            ],
            [
                (-1.0, -1.0, 1.0),
                (1.0, -1.0, 1.0),
                (1.0, 1.0, 1.0),
                (-1.0, 1.0, 1.0),
            ],
            [
                (1.0, -1.0, -1.0),
                (-1.0, -1.0, -1.0),
                (-1.0, 1.0, -1.0),
                (1.0, 1.0, -1.0),
            ],
        ];
        for face in faces {
            let corners = face.map(|(x, y, z)| corner(x, y, z));
            self.polygon(&corners, color);
        }
    }

    /// An upright box from `low` to `high`, not turned.
    pub fn block(&mut self, low: Vec3, high: Vec3, color: Vec3) {
        self.cuboid(
            (low + high) * 0.5,
            (high - low) * 0.5,
            Quat::IDENTITY,
            color,
        );
    }

    /// A beam from `from` to `to`, `thickness` square.
    pub fn beam(&mut self, from: Vec3, to: Vec3, thickness: f32, color: Vec3) {
        let along = to - from;
        let turn = Quat::from_rotation_arc(Vec3::X, along.normalize_or(Vec3::X));
        let half = Vec3::new(along.length() * 0.5, thickness * 0.5, thickness * 0.5);
        self.cuboid((from + to) * 0.5, half, turn, color);
    }

    /// An upright cylinder of `sides` sides on `base`, with its top.
    pub fn cylinder(&mut self, base: Vec3, radius: f32, height: f32, sides: u32, color: Vec3) {
        let ring = |y: f32| -> Vec<Vec3> {
            (0..sides)
                .map(|side| {
                    let angle = side as f32 / sides as f32 * std::f32::consts::TAU;
                    base + Vec3::new(angle.cos() * radius, y, -angle.sin() * radius)
                })
                .collect()
        };
        let (bottom, top) = (ring(0.0), ring(height));
        for side in 0..sides as usize {
            let next = (side + 1) % sides as usize;
            self.polygon(&[bottom[side], bottom[next], top[next], top[side]], color);
        }
        self.polygon(&top, color * 1.08);
    }

    /// An upright cone of `sides` sides on `base`, without its base.
    pub fn cone(&mut self, base: Vec3, radius: f32, height: f32, sides: u32, color: Vec3) {
        let apex = base + Vec3::Y * height;
        for side in 0..sides {
            let corner = |side: u32| {
                let angle = side as f32 / sides as f32 * std::f32::consts::TAU;
                base + Vec3::new(angle.cos() * radius, 0.0, -angle.sin() * radius)
            };
            self.polygon(&[corner(side), corner(side + 1), apex], color);
        }
    }

    /// A flat disc of `sides` sides at `centre`, facing up.
    pub fn disc(&mut self, centre: Vec3, radius: f32, sides: u32, color: Vec3) {
        let corners: Vec<Vec3> = (0..sides)
            .map(|side| {
                let angle = side as f32 / sides as f32 * std::f32::consts::TAU;
                centre + Vec3::new(angle.cos() * radius, 0.0, -angle.sin() * radius)
            })
            .collect();
        self.polygon(&corners, color);
    }

    /// A prism: the flat convex polygon `profile`, in order, carried `across`, with both its ends and its sides.
    pub fn prism(&mut self, profile: &[Vec3], across: Vec3, color: Vec3) {
        let mut profile = profile.to_vec();
        if (profile[1] - profile[0])
            .cross(profile[2] - profile[0])
            .dot(across)
            < 0.0
        {
            profile.reverse();
        }
        let far: Vec<Vec3> = profile.iter().map(|corner| *corner + across).collect();
        let near: Vec<Vec3> = profile.iter().rev().copied().collect();
        self.polygon(&near, color);
        self.polygon(&far, color);
        for side in 0..profile.len() {
            let next = (side + 1) % profile.len();
            self.polygon(&[profile[side], profile[next], far[next], far[side]], color);
        }
    }

    /// A slab standing on `outline`, anticlockwise seen from above, up to `top`, a corner of it above each of the
    /// outline's, smaller for a bevelled edge: its sloping sides and its top, without its base.
    pub fn slab(&mut self, outline: &[Vec3], top: &[Vec3], color: Vec3) {
        let sides = outline.len();
        for side in 0..sides {
            let next = (side + 1) % sides;
            self.polygon(
                &[outline[side], outline[next], top[next], top[side]],
                color * 0.85,
            );
        }
        self.polygon(top, color);
    }

    /// What has been built, as a primitive drawn with material `material`.
    pub fn primitive(self, material: usize) -> Primitive {
        Primitive::new(self.positions, self.normals, self.indices)
            .with_colors(self.colors)
            .with_material(material)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn faces_are_flat_and_face_outwards() {
        let mut shapes = Shapes::new();
        shapes.block(Vec3::ZERO, Vec3::ONE, Vec3::ONE);
        shapes.cylinder(Vec3::X * 3.0, 0.5, 1.0, 6, Vec3::ONE);
        shapes.cone(Vec3::X * 6.0, 0.5, 1.0, 5, Vec3::ONE);
        shapes.beam(Vec3::Z * 3.0, Vec3::new(1.0, 1.0, 3.0), 0.1, Vec3::ONE);
        shapes.disc(Vec3::X * 9.0, 0.5, 8, Vec3::ONE);
        let primitive = shapes.primitive(0);
        assert_eq!(primitive.material, Some(0));
        assert_eq!(primitive.positions.len(), primitive.normals.len());
        assert_eq!(primitive.indices.len() % 3, 0);
        // Every face of the block faces away from its middle.
        for (position, normal) in primitive.positions[..24]
            .iter()
            .zip(&primitive.normals[..24])
        {
            assert!((*position - Vec3::splat(0.5)).dot(*normal) > 0.0);
        }
        // The top of a disc faces up, and colours vary a little but stay near the colour asked for.
        assert_eq!(*primitive.normals.last().unwrap(), Vec3::Y);
        assert!(
            primitive
                .colors
                .iter()
                .all(|color| (color.x - 1.0).abs() < 0.15 && color.w == 1.0)
        );
        assert!(primitive.colors.windows(2).any(|pair| pair[0] != pair[1]));
    }

    #[test]
    fn prisms_and_slabs_face_outwards_whichever_way_their_outline_runs() {
        let square = [
            Vec3::ZERO,
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, -1.0),
            Vec3::new(0.0, 0.0, -1.0),
        ];
        let middle = Vec3::new(0.5, 0.5, -0.5);
        let reversed: Vec<Vec3> = square.iter().rev().copied().collect();
        for profile in [&square[..], &reversed] {
            let mut shapes = Shapes::new();
            shapes.prism(profile, Vec3::Y, Vec3::ONE);
            let prism = shapes.primitive(0);
            // Both its ends and its four sides, each facing away from its middle.
            assert_eq!(prism.indices.len() / 3, 12);
            for (position, normal) in prism.positions.iter().zip(&prism.normals) {
                assert!((*position - middle).dot(*normal) > 0.0);
            }
        }
        // A slab's top faces up, and its sides out and up, its top smaller than its outline.
        let mut shapes = Shapes::new();
        let top = square
            .map(|corner| corner + (Vec3::new(0.5, 0.0, -0.5) - corner) * 0.2 + Vec3::Y * 0.1);
        shapes.slab(&square, &top, Vec3::ONE);
        let slab = shapes.primitive(0);
        assert_eq!(slab.indices.len() / 3, 10);
        assert!(slab.normals.iter().all(|normal| normal.y > 0.0));
        assert_eq!(*slab.normals.last().unwrap(), Vec3::Y);
    }
}
