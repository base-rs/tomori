// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Low-poly shapes, built in code, as the farmyard's scenery is: each face flat, with a normal and a colour of its own, a
//! little varied face by face, so that the facets read as a hand-made model's do.
use glam::{Quat, Vec2, Vec3, Vec4};
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
    /// The faces drawn as ground, which participants walk on, each a triangle, and whether what is drawn now is.
    ground: Vec<[Vec3; 3]>,
    marking: bool,
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
        self.face(corners, normal, color);
    }

    /// A flat polygon through `corners`, of `normal` and `color`, fanned from its first corner: one face, or a piece of
    /// one, whose colour is varied already.
    fn face(&mut self, corners: &[Vec3], normal: Vec3, color: Vec4) {
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
        if self.marking {
            self.ground.extend(fan(corners));
        }
    }

    /// A level polygon through `corners`, facing up whichever way they wind, as [`Shapes::polygon`] draws it so, one face
    /// of one normal and colour, walked on as the same fan if it is ground, but drawn in triangles about `spacing` across
    /// rather than in slivers fanned from its first corner: each edge cut at the points `along` gives it, in order from
    /// its first corner to its last, then a constrained Delaunay triangulation of them refined within, its edges kept as
    /// they are cut, so that two faces whose shared edge `along` cuts alike meet corner to corner, and a chunk of it is
    /// no larger than a chunk.
    pub fn finely(
        &mut self,
        corners: &[Vec3],
        along: impl Fn(Vec3, Vec3) -> Vec<Vec3>,
        spacing: f32,
        color: Vec3,
    ) {
        use spade::{
            AngleLimit, ConstrainedDelaunayTriangulation, Point2, RefinementParameters,
            Triangulation,
        };
        let corners: Vec<Vec3> = if (corners[1] - corners[0]).cross(corners[2] - corners[0]).y < 0.0
        {
            corners.iter().rev().copied().collect()
        } else {
            corners.to_vec()
        };
        let corners = corners.as_slice();
        let normal = (corners[1] - corners[0])
            .cross(corners[2] - corners[0])
            .normalize_or_zero();
        let color = varied(color, self.faces, 0.06);
        self.faces += 1;
        // Walked on as the fan it was, not as the faces it is drawn in.
        if self.marking {
            self.ground.extend(fan(corners));
        }
        let marking = std::mem::replace(&mut self.marking, false);
        let edge: Vec<Vec3> = (0..corners.len())
            .flat_map(|index| {
                let (from, to) = (corners[index], corners[(index + 1) % corners.len()]);
                std::iter::once(from).chain(along(from, to))
            })
            .collect();
        // Where each point of its edge is, to the bit, to find it again from the triangulation's own.
        let key = |x: f64, z: f64| [(x as f32).to_bits(), (z as f32).to_bits()];
        let points: std::collections::HashMap<_, Vec3> = edge
            .iter()
            .map(|point| (key(f64::from(point.x), f64::from(point.z)), *point))
            .collect();
        let constraints = (0..edge.len())
            .map(|index| [index, (index + 1) % edge.len()])
            .collect();
        let mut triangulation: ConstrainedDelaunayTriangulation<Point2<f64>> =
            ConstrainedDelaunayTriangulation::bulk_load_cdt(
                edge.iter()
                    .map(|point| Point2::new(f64::from(point.x), f64::from(point.z)))
                    .collect(),
                constraints,
            )
            .expect("a level polygon's corners are finite and its edges do not cross");
        let refined = triangulation.refine(
            RefinementParameters::new()
                .with_angle_limit(AngleLimit::from_deg(20.0))
                .with_max_allowed_area(f64::from(spacing * spacing) * 0.5)
                .with_max_additional_vertices(edge.len() * 64)
                .keep_constraint_edges()
                .exclude_outer_faces(true),
        );
        let outer: std::collections::HashSet<_> = refined.excluded_faces.into_iter().collect();
        let level = corners[0].y;
        for face in triangulation
            .inner_faces()
            .filter(|face| !outer.contains(&face.fix()))
        {
            let [a, b, c] = face.positions().map(|point| {
                points
                    .get(&key(point.x, point.y))
                    .copied()
                    .unwrap_or(Vec3::new(point.x as f32, level, point.y as f32))
            });
            // The triangulation's faces wind anticlockwise across the ground, facing down: turned to face up.
            self.face(&[a, c, b], normal, color);
        }
        self.marking = marking;
    }

    /// Draw what `draw` draws as ground, too: the faces participants walk on, as Tomori derives where they may land
    /// from them, less what stands on them, so that what is walked on is what is drawn.
    pub fn ground(&mut self, draw: impl FnOnce(&mut Self)) {
        let marking = std::mem::replace(&mut self.marking, true);
        draw(self);
        self.marking = marking;
    }

    /// The faces drawn as ground so far, each a triangle.
    pub fn walkable(&self) -> &[[Vec3; 3]] {
        &self.ground
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

/// The part of the flat, convex polygon `corners` where `side` is no less than nothing, `side` being linear across it,
/// as how far a point is to one side of a line on the ground, or above a height: its corners there, and a corner where
/// each of its edges crosses, found along the edge; fewer than three if none of it is.
pub fn clipped(corners: &[Vec3], side: impl Fn(Vec3) -> f32) -> Vec<Vec3> {
    let mut kept = Vec::with_capacity(corners.len() + 1);
    for (index, corner) in corners.iter().enumerate() {
        let next = corners[(index + 1) % corners.len()];
        let (here, there) = (side(*corner), side(next));
        if here >= 0.0 {
            kept.push(*corner);
        }
        // Crossing between them, rather than at either, which is kept already.
        if (here > 0.0 && there < 0.0) || (here < 0.0 && there > 0.0) {
            kept.push(corner.lerp(next, here / (here - there)));
        }
    }
    kept
}

/// How far `point` is inside the edge of the convex `outline` on the ground from `from` to `to`, by its winding: more
/// than nothing within it, less beyond.
fn inside(outline: &[Vec2], from: Vec2, to: Vec2, point: Vec3) -> f32 {
    let winding: f32 = (0..outline.len())
        .map(|index| outline[index].perp_dot(outline[(index + 1) % outline.len()]))
        .sum();
    (to - from).perp_dot(Vec2::new(point.x, point.z) - from) * winding.signum()
}

/// Whether the polygon `corners` is a face rather than a sliver: three corners or more, enclosing a square
/// centimetre or more seen from above.
fn face(corners: &[Vec3]) -> bool {
    let flat: Vec<Vec2> = corners
        .iter()
        .map(|corner| Vec2::new(corner.x, corner.z))
        .collect();
    let area: f32 = (0..flat.len())
        .map(|index| flat[index].perp_dot(flat[(index + 1) % flat.len()]))
        .sum::<f32>()
        .abs()
        * 0.5;
    corners.len() >= 3 && area > 1e-4
}

/// The flat, convex polygon `corners` cut along the edges of the convex `outline` on the ground: the part within it,
/// if any, and the parts beyond it, each convex, so that together they are the polygon, and none overlaps another.
pub fn cut(corners: &[Vec3], outline: &[Vec2]) -> (Option<Vec<Vec3>>, Vec<Vec<Vec3>>) {
    let mut within = corners.to_vec();
    let mut beyond = Vec::new();
    for index in 0..outline.len() {
        let (from, to) = (outline[index], outline[(index + 1) % outline.len()]);
        let side = |point: Vec3| inside(outline, from, to, point);
        let outside = clipped(&within, |point| -side(point));
        if face(&outside) {
            beyond.push(outside);
        }
        within = clipped(&within, side);
        // None of it within: the polygon whole, rather than cut along lines that miss the outline.
        if !face(&within) {
            return (None, vec![corners.to_vec()]);
        }
    }
    (Some(within), beyond)
}

/// The points that cut the edge from `from` to `to` into pieces of the same length, at most `spacing` long, in order from
/// `from`, but neither end: none for an edge no longer.
pub fn even(from: Vec3, to: Vec3, spacing: f32) -> Vec<Vec3> {
    let pieces = (from.distance(to) / spacing).ceil().max(1.0) as usize;
    (1..pieces)
        .map(|piece| from.lerp(to, piece as f32 / pieces as f32))
        .collect()
}

/// The triangles a flat, convex polygon is drawn in, a fan about its first corner, each wound as it is.
pub fn fan(corners: &[Vec3]) -> impl Iterator<Item = [Vec3; 3]> + '_ {
    (1..corners.len().saturating_sub(1))
        .map(|index| [corners[0], corners[index], corners[index + 1]])
}

/// The triangle `corners`, wound anticlockwise seen from above, so that it faces up.
pub fn upward([a, b, c]: [Vec3; 3]) -> [Vec3; 3] {
    if (b - a).cross(c - a).y < 0.0 {
        [a, c, b]
    } else {
        [a, b, c]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_polygon_is_cut_along_an_outline_into_the_part_within_and_the_parts_beyond() {
        let square = [
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(2.0, 1.0, 0.0),
            Vec3::new(2.0, 1.0, 2.0),
            Vec3::new(0.0, 0.0, 2.0),
        ];
        let area = |corners: &[Vec3]| -> f32 {
            fan(corners)
                .map(|[a, b, c]| {
                    (Vec2::new(b.x - a.x, b.z - a.z))
                        .perp_dot(Vec2::new(c.x - a.x, c.z - a.z))
                        .abs()
                        * 0.5
                })
                .sum()
        };
        // An outline across its middle, either way round: the part within it and the two either side, which together
        // are the square, each corner on its plane.
        let outline = [
            Vec2::new(0.5, -1.0),
            Vec2::new(1.5, -1.0),
            Vec2::new(1.5, 3.0),
            Vec2::new(0.5, 3.0),
        ];
        let reversed: Vec<Vec2> = outline.iter().rev().copied().collect();
        for outline in [&outline[..], &reversed] {
            let (within, beyond) = cut(&square, outline);
            let within = within.unwrap();
            assert!((area(&within) - 2.0).abs() < 1e-5);
            assert_eq!(beyond.len(), 2);
            assert!((beyond.iter().map(|part| area(part)).sum::<f32>() - 2.0).abs() < 1e-5);
            for corner in within.iter().chain(beyond.iter().flatten()) {
                assert!((corner.y - corner.x * 0.5).abs() < 1e-6, "{corner}");
            }
            assert!(
                within
                    .iter()
                    .all(|corner| (0.5 - 1e-6..=1.5 + 1e-6).contains(&corner.x))
            );
        }
        // An outline it lies wholly within, or wholly beyond.
        let around = [
            Vec2::new(-1.0, -1.0),
            Vec2::new(3.0, -1.0),
            Vec2::new(3.0, 3.0),
            Vec2::new(-1.0, 3.0),
        ];
        let (within, beyond) = cut(&square, &around);
        assert_eq!(within.unwrap(), square);
        assert!(beyond.is_empty());
        let away = around.map(|corner| corner + Vec2::X * 10.0);
        let (within, beyond) = cut(&square, &away);
        assert!(within.is_none());
        assert_eq!(beyond, [square.to_vec()]);
        // Below a height, by the same clipping.
        let low = clipped(&square, |point| 0.5 - point.y);
        assert!((area(&low) - 2.0).abs() < 1e-5);
        assert_eq!(fan(&square).count(), 2);
        assert_eq!(fan(&[]).count(), 0);
        // Beside the outline, but within none of it, whole.
        let beside = [
            Vec2::new(1.0, 2.5),
            Vec2::new(3.0, 2.5),
            Vec2::new(3.0, 3.0),
            Vec2::new(1.0, 3.0),
        ];
        let (within, beyond) = cut(&square, &beside);
        assert!(within.is_none());
        assert_eq!(beyond, [square.to_vec()]);
        // Either way round, it faces up.
        for triangle in [
            [square[0], square[2], square[1]],
            [square[0], square[1], square[2]],
        ] {
            let [a, b, c] = upward(triangle);
            assert!((b - a).cross(c - a).y > 0.0);
            assert_eq!(a, square[0]);
        }
    }

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
    fn only_what_is_drawn_as_ground_is_walked_on() {
        let mut shapes = Shapes::new();
        shapes.block(Vec3::ZERO, Vec3::ONE, Vec3::ONE);
        shapes.ground(|shapes| {
            shapes.disc(Vec3::X * 3.0, 0.5, 6, Vec3::ONE);
            // Within, still ground, and after it, too.
            shapes.ground(|shapes| shapes.disc(Vec3::X * 5.0, 0.5, 4, Vec3::ONE));
            shapes.disc(Vec3::X * 7.0, 0.5, 3, Vec3::ONE);
        });
        shapes.disc(Vec3::X * 9.0, 0.5, 8, Vec3::ONE);
        // The hexagon's four triangles, the square's two and the triangle, as drawn, and nothing of the block.
        let ground = shapes.walkable();
        assert_eq!(ground.len(), 4 + 2 + 1);
        assert!(
            ground
                .iter()
                .flatten()
                .all(|corner| corner.y == 0.0 && corner.x > 2.0 && corner.x < 8.0)
        );
        let drawn = shapes.primitive(0);
        assert_eq!(drawn.indices.len() / 3, 12 + 4 + 2 + 1 + 6);
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
