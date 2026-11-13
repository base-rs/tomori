// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The yard, built in code as one model: the meadow, the packed earth inside the fence, the fence and its gate posts,
//! the water trough, the feeder and its scattered grain, the hay bales, the pines beyond, and the lantern on the gate
//! post, whose glass glows. The coop is a model of its own (see [`crate::coop`]), and so is the land beyond the pines
//! (see [`crate::backdrop`]).
use glam::{Quat, Vec2, Vec3, Vec4};
use tomori_assets::{Limits, Material, Mesh, Model, Node, Transform};

use crate::layout;
use crate::shapes::Shapes;

/// The farmyard's palette, linear RGB, for an evening under the moon.
pub mod palette {
    use glam::Vec3;

    pub const GRASS: Vec3 = Vec3::new(0.10, 0.17, 0.07);
    pub const EARTH: Vec3 = Vec3::new(0.26, 0.17, 0.10);
    pub const WOOD: Vec3 = Vec3::new(0.34, 0.22, 0.13);
    pub const DARK_WOOD: Vec3 = Vec3::new(0.20, 0.12, 0.07);
    pub const WEATHERED: Vec3 = Vec3::new(0.42, 0.36, 0.30);
    pub const RED: Vec3 = Vec3::new(0.42, 0.09, 0.06);
    pub const ROOF: Vec3 = Vec3::new(0.23, 0.13, 0.11);
    pub const STRAW: Vec3 = Vec3::new(0.62, 0.48, 0.20);
    pub const GRAIN: Vec3 = Vec3::new(0.70, 0.55, 0.25);
    pub const WATER: Vec3 = Vec3::new(0.10, 0.16, 0.22);
    pub const METAL: Vec3 = Vec3::new(0.30, 0.31, 0.33);
    pub const PINE: Vec3 = Vec3::new(0.05, 0.11, 0.07);
    pub const BARK: Vec3 = Vec3::new(0.15, 0.10, 0.07);
    pub const SHADOW: Vec3 = Vec3::new(0.03, 0.025, 0.02);
    /// A lantern's warm glow, which its glass gives off.
    pub const GLOW: Vec3 = Vec3::new(1.0, 0.62, 0.28);
}

/// The evening's light, linear RGB, which the scenery lights the farmyard with and the backdrop's ranges are lit by.
pub mod evening {
    use glam::Vec3;

    /// The way towards the moon, low in the east, not of unit length.
    pub const MOON: Vec3 = Vec3::new(0.62, 0.36, -0.7);
    /// The moon's light, and the sky's and the ground's.
    pub const MOONLIGHT: Vec3 = Vec3::new(0.30, 0.34, 0.46);
    pub const SKY: Vec3 = Vec3::new(0.10, 0.12, 0.2);
    pub const GROUND: Vec3 = Vec3::new(0.06, 0.05, 0.04);
}

/// The material most of the yard is drawn with, white, so that its faces' own colours show.
pub const PLAIN: usize = 0;
/// The material of a lantern's glass, which glows.
pub const GLASS: usize = 1;

/// The materials of the farmyard's models: [`PLAIN`] and [`GLASS`].
pub fn materials() -> Vec<Material> {
    vec![
        Material::new("plain", Vec4::ONE),
        Material::new("glass", Vec4::new(1.0, 0.85, 0.6, 1.0)).with_emissive(palette::GLOW * 1.6),
    ]
}

/// A deterministic number from 0 up to 1, for scattering things alike everywhere.
pub fn scatter(seed: u32) -> f32 {
    let mut hash = seed.wrapping_mul(0x2c1b_3c6d) ^ 0x297a_2d39;
    hash ^= hash >> 13;
    hash = hash.wrapping_mul(0x7fb5_d329);
    hash ^= hash >> 16;
    (hash & 0xffff) as f32 / 65536.0
}

/// The path's stones, a little paler than the earth they are set in.
const STONE: Vec3 = Vec3::new(0.32, 0.28, 0.23);

/// A lantern hung at `at`: its cap and base, and its glass, which glows, into `glass`.
pub fn lantern(shapes: &mut Shapes, glass: &mut Shapes, at: Vec3) {
    shapes.block(
        at + Vec3::new(-0.09, 0.11, -0.09),
        at + Vec3::new(0.09, 0.15, 0.09),
        palette::METAL,
    );
    shapes.block(
        at + Vec3::new(-0.08, -0.14, -0.08),
        at + Vec3::new(0.08, -0.11, 0.08),
        palette::METAL,
    );
    glass.block(
        at + Vec3::new(-0.065, -0.11, -0.065),
        at + Vec3::new(0.065, 0.11, 0.065),
        palette::GLOW,
    );
}

/// The path of flat stones from the gate towards the coop and the feeder ([`layout::stones`]), each standing a little
/// proud of the earth, its edge bevelled.
fn path(shapes: &mut Shapes) {
    for stone in layout::stones() {
        let ground = |corner: &glam::Vec2| Vec3::new(corner.x, 0.0, corner.y);
        let outline: Vec<Vec3> = stone.outline.iter().map(ground).collect();
        let top: Vec<Vec3> = stone
            .top()
            .iter()
            .map(|corner| ground(corner) + Vec3::Y * stone.height)
            .collect();
        shapes.slab(&outline, &top, STONE);
    }
}

fn fence_around(shapes: &mut Shapes) {
    let fence = layout::FENCE;
    let height = layout::FENCE_HEIGHT;
    let corners = [
        Vec3::new(fence.low.x, 0.0, fence.high.y),
        Vec3::new(fence.low.x, 0.0, fence.low.y),
        Vec3::new(fence.high.x, 0.0, fence.low.y),
        Vec3::new(fence.high.x, 0.0, fence.high.y),
    ];
    // Three sides, then the front, either side of the gate.
    let mut runs = vec![
        (corners[0], corners[1]),
        (corners[1], corners[2]),
        (corners[2], corners[3]),
    ];
    runs.push((corners[3], Vec3::new(layout::GATE[1], 0.0, fence.high.y)));
    runs.push((Vec3::new(layout::GATE[0], 0.0, fence.high.y), corners[0]));
    for (index, (from, to)) in runs.into_iter().enumerate() {
        let posts = ((to - from).length() / 1.6).ceil().max(1.0) as u32;
        for post in 0..=posts {
            let at = from.lerp(to, post as f32 / posts as f32);
            let lean = (scatter(index as u32 * 97 + post) - 0.5) * 0.06;
            shapes.cuboid(
                at + Vec3::Y * height * 0.5,
                Vec3::new(
                    layout::FENCE_POST * 0.5,
                    height * 0.5,
                    layout::FENCE_POST * 0.5,
                ),
                Quat::from_rotation_z(lean),
                palette::WEATHERED * (0.9 + scatter(post) * 0.2),
            );
        }
        for rail in [0.38, 0.8] {
            shapes.beam(
                from + Vec3::Y * rail,
                to + Vec3::Y * rail,
                0.07,
                palette::WEATHERED,
            );
        }
    }
    // The gate's posts, taller, and the lantern's arm.
    let gate = layout::GATE_POST * 0.5;
    for x in layout::GATE {
        shapes.block(
            Vec3::new(x - gate, 0.0, fence.high.y - gate),
            Vec3::new(x + gate, layout::GATE_HEIGHT, fence.high.y + gate),
            palette::DARK_WOOD,
        );
    }
    let arm = Vec3::new(layout::GATE[1], height * 1.3, fence.high.y);
    shapes.beam(
        arm,
        Vec3::new(layout::GATE_LANTERN.x, arm.y, arm.z),
        0.04,
        palette::DARK_WOOD,
    );
}

fn trough(shapes: &mut Shapes) {
    let trough = layout::TROUGH;
    let top = layout::TROUGH_HEIGHT;
    let wall = 0.05;
    let (low, high) = (trough.low, trough.high);
    shapes.block(
        Vec3::new(low.x, 0.08, low.y),
        Vec3::new(high.x, 0.13, high.y),
        palette::DARK_WOOD,
    );
    shapes.block(
        Vec3::new(low.x, 0.13, low.y),
        Vec3::new(low.x + wall, top, high.y),
        palette::WOOD,
    );
    shapes.block(
        Vec3::new(high.x - wall, 0.13, low.y),
        Vec3::new(high.x, top, high.y),
        palette::WOOD,
    );
    shapes.block(
        Vec3::new(low.x, 0.13, low.y),
        Vec3::new(high.x, top, low.y + wall),
        palette::WOOD,
    );
    shapes.block(
        Vec3::new(low.x, 0.13, high.y - wall),
        Vec3::new(high.x, top, high.y),
        palette::WOOD,
    );
    shapes.polygon(
        &[
            Vec3::new(low.x + wall, top - 0.06, high.y - wall),
            Vec3::new(high.x - wall, top - 0.06, high.y - wall),
            Vec3::new(high.x - wall, top - 0.06, low.y + wall),
            Vec3::new(low.x + wall, top - 0.06, low.y + wall),
        ],
        palette::WATER,
    );
    for x in [low.x + 0.12, high.x - 0.12] {
        shapes.block(
            Vec3::new(x - 0.04, 0.0, low.y + 0.04),
            Vec3::new(x + 0.04, 0.08, high.y - 0.04),
            palette::DARK_WOOD,
        );
    }
}

fn feeder(shapes: &mut Shapes) {
    let at = Vec3::new(layout::FEEDER.x, 0.0, layout::FEEDER.y);
    // Its grain heaped a little on it, rather than a disc a millimetre above its top, which flickered against it.
    shapes.cylinder(at, layout::FEEDER_RADIUS, 0.06, 10, palette::METAL);
    shapes.cone(
        at + Vec3::Y * 0.06,
        layout::FEEDER_RADIUS * 0.85,
        0.035,
        10,
        palette::GRAIN,
    );
    // Grain scattered about it, thinning outwards, each a little above any it overlaps.
    let mut layers = Layers::default();
    for grain in 0..90 {
        let angle = scatter(grain * 3) * std::f32::consts::TAU;
        let distance =
            layout::FEEDER_RADIUS + scatter(grain * 3 + 1).powi(2) * layout::GRAIN_RADIUS;
        let size = 0.012 + scatter(grain * 3 + 2) * 0.01;
        let middle = at + Vec3::new(angle.cos() * distance, 0.0, angle.sin() * distance);
        let top = layers.over(middle, Vec2::splat(size));
        let centre = middle + Vec3::Y * top;
        shapes.block(
            centre - Vec3::splat(size),
            centre + Vec3::new(size, 0.0, size),
            palette::GRAIN,
        );
    }
}

/// Small flat things scattered on the earth, each laid 1.2 mm above any earlier one it overlaps, from 6 mm up, so
/// that none flickers against the earth or another.
#[derive(Default)]
struct Layers {
    laid: Vec<(Vec2, Vec2, u32)>,
}

impl Layers {
    /// The height of the top of the next, reaching `reach` either way across the ground about `middle`.
    fn over(&mut self, middle: Vec3, reach: Vec2) -> f32 {
        let middle = Vec2::new(middle.x, middle.z);
        let (low, high) = (middle - reach, middle + reach);
        let layer = (0..)
            .find(|layer| {
                !self.laid.iter().any(|(other_low, other_high, other)| {
                    other == layer && low.cmplt(*other_high).all() && other_low.cmplt(high).all()
                })
            })
            .expect("a layer");
        self.laid.push((low, high, layer));
        0.006 + layer as f32 * 0.0012
    }
}

fn hay(shapes: &mut Shapes) {
    let hay = layout::HAY;
    let (low, high) = (hay.low, hay.high);
    let mid = (low.x + high.x) * 0.5;
    let bale = |shapes: &mut Shapes, low: Vec3, high: Vec3| {
        shapes.block(low, high, palette::STRAW);
        for band in [0.3, 0.7] {
            let x = low.x + (high.x - low.x) * band;
            shapes.block(
                Vec3::new(x - 0.015, low.y - 0.005, low.z - 0.005),
                Vec3::new(x + 0.015, high.y + 0.005, high.z + 0.005),
                palette::DARK_WOOD,
            );
        }
    };
    bale(
        shapes,
        Vec3::new(low.x, 0.0, low.y),
        Vec3::new(mid - 0.02, 0.42, low.y + 0.55),
    );
    bale(
        shapes,
        Vec3::new(mid + 0.02, 0.0, low.y),
        Vec3::new(high.x, 0.42, low.y + 0.55),
    );
    bale(
        shapes,
        Vec3::new(low.x + 0.35, 0.42, low.y + 0.02),
        Vec3::new(high.x - 0.35, 0.84, low.y + 0.53),
    );
    bale(
        shapes,
        Vec3::new(low.x + 0.1, 0.0, high.y - 0.55),
        Vec3::new(mid + 0.2, 0.42, high.y),
    );
    // Loose straw strewn in front, each strand a little above any it crosses, as the grain is.
    let mut layers = Layers::default();
    for straw in 0..40 {
        let at = Vec3::new(
            low.x + scatter(straw * 5 + 400) * (high.x - low.x),
            0.0,
            high.y + 0.1 + scatter(straw * 5 + 401) * 0.6,
        );
        let angle = scatter(straw * 5 + 402) * std::f32::consts::TAU;
        let (sin, cos) = (angle.sin().abs(), angle.cos().abs());
        let reach = Vec2::new(0.09 * cos + 0.006 * sin, 0.09 * sin + 0.006 * cos);
        let top = layers.over(at, reach);
        shapes.cuboid(
            at + Vec3::Y * (top - 0.004),
            Vec3::new(0.09, 0.004, 0.006),
            Quat::from_rotation_y(angle),
            palette::STRAW,
        );
    }
}

/// The pines beyond the fence, each where its trunk stands on the ground and how tall it is: none behind home's view of
/// the yard's gate, so that the way in stays open, nor in the river or the pool, nor before the waterfall and the
/// river where participants see them from (see [`crate::backdrop::open`]).
pub fn pines() -> Vec<(Vec3, f32)> {
    (0..14)
        .filter_map(|tree| {
            let angle = (tree as f32 / 14.0 + scatter(tree * 7) * 0.05) * std::f32::consts::TAU;
            let distance = 13.0 + scatter(tree * 7 + 1) * 10.0;
            let at = glam::Vec2::new(angle.cos() * distance, angle.sin() * distance - 4.0);
            let height = 4.0 + scatter(tree * 7 + 2) * 3.5;
            let gate = at.y > 4.0 && at.x.abs() < 6.0;
            (!gate && crate::backdrop::open(at, height * 0.32, height)).then(|| {
                // Sunk a little, so that none stands proud of a slope.
                let ground = crate::backdrop::ground(at) - 0.15;
                (Vec3::new(at.x, ground, at.y), height)
            })
        })
        .collect()
}

fn trees(shapes: &mut Shapes) {
    for (base, height) in pines() {
        shapes.cylinder(base, 0.18, height * 0.25, 5, palette::BARK);
        for (layer, (from, radius)) in [(0.18, 1.0), (0.42, 0.78), (0.64, 0.55)]
            .into_iter()
            .enumerate()
        {
            shapes.cone(
                base + Vec3::Y * height * from,
                radius * height * 0.32,
                height * 0.45,
                7,
                palette::PINE * (1.0 + layer as f32 * 0.12),
            );
        }
    }
}

/// The notice board against the right of the fence: a wooden back on two posts, framed in darker wood, with a plank
/// across its top and a little lean-to roof over it, sloping down towards the yard, as a farm's notice board stands
/// against its fence. The farmyard's own projection wall stands on its back (see [`layout::NOTICEBOARD`]), so that what people
/// put on it reads as part of the place.
fn noticeboard(shapes: &mut Shapes) {
    let [low, high] = layout::NOTICEBOARD_BACK.map(Vec3::from_array);
    let post = layout::NOTICEBOARD_POST * 0.5;
    let top = layout::NOTICEBOARD_HEIGHT - 0.12;
    for [x, z] in layout::NOTICEBOARD_POSTS {
        shapes.block(
            Vec3::new(x - post, 0.0, z - post),
            Vec3::new(x + post, top, z + post),
            palette::DARK_WOOD,
        );
    }
    shapes.block(low, high, palette::WOOD);
    // A frame of darker wood about the back, proud of it, outside where the wall stands.
    let (proud, trim) = (0.012, 0.05);
    let front = low.x - proud;
    for (from, to) in [
        (
            Vec3::new(front, low.y, low.z),
            Vec3::new(low.x, low.y + trim, high.z),
        ),
        (
            Vec3::new(front, high.y - trim, low.z),
            Vec3::new(low.x, high.y, high.z),
        ),
        (
            Vec3::new(front, low.y, low.z),
            Vec3::new(low.x, high.y, low.z + trim),
        ),
        (
            Vec3::new(front, low.y, high.z - trim),
            Vec3::new(low.x, high.y, high.z),
        ),
    ] {
        shapes.block(from, to, palette::DARK_WOOD);
    }
    // A plank across the top, like a sign without words, and the roof over it, overhanging the posts.
    shapes.block(
        Vec3::new(low.x - 0.01, high.y + 0.02, low.z + 0.05),
        Vec3::new(high.x, high.y + 0.1, high.z - 0.05),
        palette::WEATHERED,
    );
    let middle = (low.z + high.z) * 0.5;
    let half = (high.z - low.z) * 0.5 + 0.16;
    shapes.cuboid(
        Vec3::new(low.x - 0.12, top + 0.06, middle),
        Vec3::new(0.24, 0.025, half),
        Quat::from_rotation_z(0.28),
        palette::ROOF,
    );
}

/// The yard, as a model of one node whose mesh draws it all.
pub fn model() -> Model {
    let (shapes, glass) = yard();
    Model::built(
        vec![Node::new("yard", Transform::IDENTITY).with_mesh(0)],
        vec![0],
        vec![Mesh::new(
            "yard",
            vec![shapes.primitive(PLAIN), glass.primitive(GLASS)],
        )],
        materials(),
        &Limits::default(),
    )
    .expect("the yard is built within the limits")
}

/// What the yard draws as ground, which participants walk on: its packed earth, inside the fence, each triangle as
/// [`model`] draws it. The path's stones on it are walked over on the earth, as they stand too little proud of it to
/// hide the ring where a teleport lands.
pub fn ground() -> Vec<[Vec3; 3]> {
    yard().0.walkable().to_vec()
}

/// How far apart the corners within the meadow and the yard's earth are, which are each one flat face: a seventh of the
/// chunks the scenery draws the yard in, so that none of their triangles reaches far beyond its chunk's cell.
const FINELY: f32 = 1.0;

/// The meadow, out to the foothills' first ring, whose corners are its edge, so that the river's channel beyond it is
/// open, cut along the beach's outline, the part within it the beach's shingle, each part one face, as when each was
/// fanned from its first corner, drawn in triangles about [`FINELY`] across: each edge along one of the outline's lines
/// cut at every part's corner on that line and every [`FINELY`] along it, alike for the parts either side of it, so that
/// they meet corner to corner, and the ring's chords, about a metre long, kept whole, as the foothills meet them.
fn meadow(shapes: &mut Shapes) {
    let outline = crate::beach::outline();
    let (beach, meadow) = crate::shapes::cut(&crate::backdrop::meadow(), &outline);
    let parts: Vec<(Vec<Vec3>, Vec3)> = meadow
        .into_iter()
        .map(|part| (part, palette::GRASS))
        .chain(beach.map(|part| (part, crate::beach::shingle(0))))
        .collect();
    let lines: Vec<(Vec2, Vec2)> = (0..outline.len())
        .map(|index| (outline[index], outline[(index + 1) % outline.len()]))
        .collect();
    let flat = |point: Vec3| Vec2::new(point.x, point.z);
    let on = |(from, to): (Vec2, Vec2), point: Vec3| {
        ((to - from).perp_dot(flat(point) - from) / from.distance(to)).abs() < 1e-3
    };
    let along_line = |(from, to): (Vec2, Vec2), point: Vec3| {
        (flat(point) - from).dot(to - from) / from.distance(to)
    };
    // Each line's marks, by how far along it each is: every part's corner on it, and every `FINELY` along it but those
    // within a quarter of that of a corner.
    let marks: Vec<Vec<(f32, Vec3)>> = lines
        .iter()
        .map(|line| {
            let mut marks: Vec<(f32, Vec3)> = Vec::new();
            for corner in parts.iter().flat_map(|(part, _)| part) {
                if on(*line, *corner) && !marks.iter().any(|(_, mark)| mark == corner) {
                    marks.push((along_line(*line, *corner), *corner));
                }
            }
            let corners = marks.len();
            let (from, to) = *line;
            for step in -64..=64 {
                let at = step as f32 * FINELY;
                if marks[..corners]
                    .iter()
                    .all(|(mark, _)| (mark - at).abs() > FINELY * 0.25)
                {
                    let point = from + (to - from).normalize() * at;
                    marks.push((at, Vec3::new(point.x, crate::backdrop::MEADOW, point.y)));
                }
            }
            marks.sort_by(|a, b| a.0.total_cmp(&b.0));
            marks
        })
        .collect();
    let along = |from: Vec3, to: Vec3| -> Vec<Vec3> {
        let Some(line) = lines
            .iter()
            .position(|line| on(*line, from) && on(*line, to))
        else {
            return Vec::new();
        };
        let ends = (along_line(lines[line], from), along_line(lines[line], to));
        let (low, high) = (ends.0.min(ends.1), ends.0.max(ends.1));
        let mut within: Vec<Vec3> = marks[line]
            .iter()
            .filter(|(at, _)| *at > low + 1e-4 && *at < high - 1e-4)
            .map(|(_, mark)| *mark)
            .collect();
        if ends.0 > ends.1 {
            within.reverse();
        }
        within
    };
    for (part, color) in parts {
        shapes.finely(&part, along, FINELY, color);
    }
}

/// The yard's shapes, lit and glowing.
fn yard() -> (Shapes, Shapes) {
    let mut shapes = Shapes::new();
    let mut glass = Shapes::new();
    // The meadow, and the packed earth inside the fence, a little above it, each one face in triangles about [`FINELY`]
    // across rather than fanned from one corner across the yard, so that it is drawn in chunks of its own.
    meadow(&mut shapes);
    let fence = layout::FENCE;
    shapes.ground(|shapes| {
        shapes.finely(
            &[
                Vec3::new(fence.low.x, 0.0, fence.high.y),
                Vec3::new(fence.high.x, 0.0, fence.high.y),
                Vec3::new(fence.high.x, 0.0, fence.low.y),
                Vec3::new(fence.low.x, 0.0, fence.low.y),
            ],
            |from, to| crate::shapes::even(from, to, FINELY),
            FINELY,
            palette::EARTH,
        )
    });
    path(&mut shapes);
    fence_around(&mut shapes);
    trough(&mut shapes);
    feeder(&mut shapes);
    hay(&mut shapes);
    noticeboard(&mut shapes);
    crate::workbench::build(&mut shapes, &mut glass);
    trees(&mut shapes);
    lantern(&mut shapes, &mut glass, layout::GATE_LANTERN);
    (shapes, glass)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_yard_is_one_checked_model_whose_lantern_glows() {
        let model = model();
        assert_eq!(model.node("yard"), Some(0));
        let primitives = &model.meshes[0].primitives;
        assert_eq!(primitives.len(), 2);
        assert!(primitives[0].positions.len() > 1000);
        assert_eq!(model.materials[GLASS].emissive, palette::GLOW * 1.6);
        // Everything but the meadow and the trees is inside the fence.
        let (low, high) = primitives[1].bounds();
        assert!(
            layout::FENCE
                .grown(0.5)
                .contains(glam::Vec2::new(low.x, low.z))
        );
        assert!(
            layout::FENCE
                .grown(0.5)
                .contains(glam::Vec2::new(high.x, high.z))
        );
        assert!(
            (0..100)
                .map(scatter)
                .all(|value| (0.0..1.0).contains(&value))
        );
    }

    #[test]
    fn the_path_is_a_few_stones_of_a_few_faces_each() {
        let mut shapes = Shapes::new();
        path(&mut shapes);
        let path = shapes.primitive(PLAIN);
        // Each stone's sides and top: 159 triangles, where the flat discs it was were 45.
        assert_eq!(path.indices.len() / 3, 159);
        // Its tops face up, above the earth, and its sides slope out and up from it.
        for (position, normal) in path.positions.iter().zip(&path.normals) {
            assert!(position.y >= 0.0 && position.y <= 0.02);
            assert!(normal.y > 0.3, "{normal}");
        }
    }

    #[test]
    fn the_earth_and_meadow_are_drawn_in_small_triangles_and_walked_on_as_the_earth_was() {
        // Walked on as the one face of the earth inside the fence, fanned from its first corner.
        let fence = layout::FENCE;
        let [a, b, c, d] = [
            Vec3::new(fence.low.x, 0.0, fence.high.y),
            Vec3::new(fence.high.x, 0.0, fence.high.y),
            Vec3::new(fence.high.x, 0.0, fence.low.y),
            Vec3::new(fence.low.x, 0.0, fence.low.y),
        ];
        assert_eq!(ground(), [[a, b, c], [a, c, d]]);
        // Drawn in triangles none more than 1.5 m across the ground, each facing up, all of one colour for each face
        // they draw, covering the earth and the meadow as they did.
        let (shapes, _) = yard();
        let primitive = shapes.primitive(PLAIN);
        let (mut earth, mut meadow) = (0.0, 0.0);
        let mut colors = std::collections::HashSet::new();
        for triangle in primitive.indices.as_chunks::<3>().0 {
            let corners = triangle.map(|corner| primitive.positions[corner as usize]);
            let level = corners[0].y;
            if !corners.iter().all(|corner| corner.y == level)
                || ![0.0, crate::backdrop::MEADOW].contains(&level)
            {
                continue;
            }
            let normal = (corners[1] - corners[0]).cross(corners[2] - corners[0]);
            let (low, high) = (
                corners[0].min(corners[1]).min(corners[2]),
                corners[0].max(corners[1]).max(corners[2]),
            );
            if normal.y <= 0.0 || (level == 0.0 && !fence.contains(glam::Vec2::new(low.x, low.z))) {
                continue;
            }
            assert!((high - low).x.max((high - low).z) <= 1.5, "{corners:?}");
            let area = normal.length() * 0.5;
            if level == 0.0 {
                earth += area;
            } else {
                meadow += area;
            }
            colors.insert((
                level.to_bits(),
                primitive.colors[triangle[0] as usize]
                    .to_array()
                    .map(f32::to_bits),
            ));
        }
        assert!(
            (earth - fence.size().x * fence.size().y).abs() < 0.01,
            "{earth}"
        );
        let ring = crate::backdrop::meadow();
        let disc: f32 = (0..ring.len())
            .map(|index| {
                let (from, to) = (ring[index], ring[(index + 1) % ring.len()]);
                (from.x * to.z - to.x * from.z) * 0.5
            })
            .sum::<f32>()
            .abs();
        assert!((meadow - disc).abs() < 0.05, "{meadow} {disc}");
        // The earth, and each of the meadow's parts and the beach's shingle within it.
        let (_, parts) = crate::shapes::cut(&ring, &crate::beach::outline());
        assert_eq!(colors.len(), 1 + parts.len() + 1);
    }

    #[test]
    fn nothing_lies_flat_just_above_the_earth_to_flicker_against_it() {
        // Every face of the yard and the coop that faces up, but the meadow and the earth themselves, stands at least
        // 5 mm above whatever flat face it is over, so that none fights the earth, the coop's floor or another for depth.
        // Within a few centimetres of the earth, and of the coop's floor, where what lies on them is seen; and no two
        // faces there that overlap lie within a millimetre of each other, but a face's own triangles.
        let coop = crate::coop::model();
        let placed = |point: Vec3| crate::coop::placement().transform_point3(point);
        let unmoved = |point: Vec3| point;
        // Each face that faces up, low down: its height, its bounds on the ground, and which face it is of which model.
        let mut flats: Vec<(f32, glam::Vec2, glam::Vec2, (usize, u32))> = Vec::new();
        for (which, (model, place)) in [
            (model(), &unmoved as &dyn Fn(Vec3) -> Vec3),
            (coop, &placed),
        ]
        .into_iter()
        .enumerate()
        {
            for (primitive_index, primitive) in model.meshes[0].primitives.iter().enumerate() {
                for triangle in primitive.indices.chunks(3) {
                    let corners = [0, 1, 2]
                        .map(|corner| place(primitive.positions[triangle[corner] as usize]));
                    let normal = (corners[1] - corners[0]).cross(corners[2] - corners[0]);
                    let height = corners[0].y;
                    let low_down = (-0.02..0.05).contains(&height)
                        || (layout::COOP_FLOOR - 0.01..layout::COOP_FLOOR + 0.06).contains(&height);
                    if normal.normalize_or_zero().y > 0.999 && low_down {
                        let flat = |corner: Vec3| glam::Vec2::new(corner.x, corner.z);
                        let low = flat(corners[0].min(corners[1]).min(corners[2]));
                        let high = flat(corners[0].max(corners[1]).max(corners[2]));
                        // A face's triangles share its first corner.
                        flats.push((
                            height,
                            low,
                            high,
                            (which * 2 + primitive_index, triangle[0]),
                        ));
                    }
                }
            }
        }
        let fence = layout::FENCE;
        let mut decals = 0;
        for (index, (height, low, high, face)) in flats.iter().enumerate() {
            let inside = fence.contains(*low) && fence.contains(*high);
            let ground = [0.0, crate::backdrop::MEADOW].contains(height);
            if (-0.02..0.05).contains(height) && inside && !ground {
                assert!(*height >= 0.005, "{height} {low} {high}");
                decals += 1;
            }
            for (other, other_low, other_high, other_face) in &flats[index + 1..] {
                let overlaps = low.cmplt(*other_high).all() && other_low.cmplt(*high).all();
                // The meadow's parts, cut along the beach's outline from one flat polygon, and the triangles each of them
                // and the earth is drawn in, meet rather than overlap.
                if overlaps && face != other_face && !(ground && height == other) {
                    assert!(
                        (height - other).abs() >= 0.001,
                        "{height} {other} {low} {high}"
                    );
                }
            }
        }
        // The path's stones, the grain about the feeder and the straw by the hay among them.
        assert!(decals > 100, "{decals}");
    }
}
