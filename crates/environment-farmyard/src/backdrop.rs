// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The land beyond the fence, a New Zealand backdrop: bush-clad foothills ringing the yard, pasture rising to them, a
//! waterfall falling down a cliff in the foothills behind the coop into a pool, the river it feeds winding round the
//! yard's right side through grey shingle, native bush, tree ferns, cabbage trees, podocarps and flax, and, beyond, the
//! ranges, bluer and hazier with distance, their highest peaks under snow.
//!
//! What is near enough to walk towards is built here in code, laid out within 50 m of the eyes from anywhere
//! participants stand, which was the headset's far plane when it was laid out and is now the bound that keeps it compact
//! and within its haze, as every client draws out to 2 km: the foothills' ground as one model ([`model`]), the plants as
//! a model for each kind, in the yard's low-poly style and palette, placed where [`plants`] says, deterministically, so
//! that every client draws the same bush, but its tree ferns, which are drawn as the ferns near the trail are
//! ([`crate::ferns::bush`]). The ranges stand far beyond that bound, so they are directions rather than places
//! ([`ranges`]), which the scenery draws at the sky's depth, behind everything; and the water is drawn by the scenery's own shader, which moves it ([`water`]). Where the
//! river, the waterfall and the bush are is in [`layout`], which [`layout::sounds`] names for ambient sound. See the
//! crate's `context/backdrop.md`.
use std::f32::consts::{PI, TAU};

use glam::{Mat4, Quat, Vec2, Vec3};
use tomori_assets::{Limits, Mesh, Model, Node, Transform};
use tomori_environment::sightlines::{Solid, Spot};

use crate::layout::{self, BACKDROP_CENTRE, POOL, POOL_RADIUS, RIVER, RIVER_HALF_WIDTH};
use crate::props::{self, PLAIN, palette, scatter};
use crate::shapes::Shapes;

mod ranges;
mod water;

#[cfg(test)]
use ranges::{FOOT, RANGES, towards};
pub use ranges::{Far, ranges};
pub use water::{FALLING, FOAM, MIST, SPREAD, SURFACE, Water, upper_water, water};

/// The most triangles the backdrop may draw, its foothills, every plant, the ranges, the water, the river's stones, the
/// waterfall rock and the trail together, so that it stays modest beside the yard on the Quest: 20,000 before the trail
/// and the stones came, which added about 2,000 in two draws.
pub const BUDGET: usize = 22_000;

/// How far from [`BACKDROP_CENTRE`] each ring of the foothills' ground is: the first just beyond the fence's corners,
/// at the meadow's level, the cliff's foot and lip among them ([`layout::CLIFF`]), and the upper cliff's behind the
/// terrace ([`layout::UPPER_CLIFF`]), and the last within the backdrop's 50 m bound from the eyes, from anywhere
/// participants land in the yard.
pub const RINGS: [f32; 17] = [
    11.5, 13.0, 14.5, 16.0, 17.5, 19.0, 20.5, 22.0, 23.5, 25.0, 26.5, 28.5, 29.5, 30.5, 32.5, 34.5,
    36.0,
];
/// How many segments each ring is drawn in, so that the waterfall's way, [`layout::WATERFALL_AZIMUTH`], is one of them.
pub const SEGMENTS: usize = 96;
/// How high the meadow is drawn, a centimetre below the yard's earth, out to the foothills' first ring, whose corners
/// are its edge's, so that the two meet without a step or a gap, and the river's channel, below it beyond, is open.
pub const MEADOW: f32 = -0.01;

/// How high the pasture rises from the meadow to the foot of the foothills.
const APRON: f32 = 1.2;

/// The colours of the land beyond the fence, linear RGB, as the yard's palette is.
pub mod colors {
    use glam::Vec3;

    /// The native bush on the foothills, its canopy, and the darker and the lighter of its trees.
    pub const BUSH: Vec3 = Vec3::new(0.04, 0.085, 0.04);
    pub const BUSH_DARK: Vec3 = Vec3::new(0.03, 0.06, 0.035);
    pub const BUSH_LIGHT: Vec3 = Vec3::new(0.06, 0.11, 0.045);
    /// The grey shingle along the river, and the rock of the waterfall's cliff.
    pub const SHINGLE: Vec3 = Vec3::new(0.21, 0.2, 0.18);
    pub const ROCK: Vec3 = Vec3::new(0.11, 0.105, 0.1);
    /// A cabbage tree's bark and its tufts of leaves.
    pub const CABBAGE_BARK: Vec3 = Vec3::new(0.14, 0.12, 0.09);
    pub const CABBAGE: Vec3 = Vec3::new(0.17, 0.21, 0.09);
    /// A podocarp's bark and its bronze-green crown.
    pub const PODOCARP_BARK: Vec3 = Vec3::new(0.16, 0.11, 0.08);
    pub const PODOCARP: Vec3 = Vec3::new(0.06, 0.085, 0.045);
    /// Flax's leaves and its flower stalk.
    pub const FLAX: Vec3 = Vec3::new(0.09, 0.14, 0.06);
    pub const STALK: Vec3 = Vec3::new(0.12, 0.07, 0.04);
    /// The ranges' rock, bluer the further they are, and their snow.
    pub const RANGE_NEAR: Vec3 = Vec3::new(0.035, 0.07, 0.05);
    pub const RANGE_MIDDLE: Vec3 = Vec3::new(0.08, 0.11, 0.13);
    pub const RANGE_FAR: Vec3 = Vec3::new(0.16, 0.18, 0.23);
    pub const SNOW: Vec3 = Vec3::new(0.66, 0.7, 0.78);
    /// The water: deep, falling and foaming.
    pub const WATER: Vec3 = Vec3::new(0.02, 0.04, 0.055);
    pub const FALLING: Vec3 = Vec3::new(0.22, 0.27, 0.32);
    pub const FOAM: Vec3 = Vec3::new(0.5, 0.55, 0.6);
    /// A river stone, paler than the shingle it lies on.
    pub const STONE: Vec3 = Vec3::new(0.27, 0.27, 0.26);
}

fn smoothstep(low: f32, high: f32, value: f32) -> f32 {
    let t = ((value - low) / (high - low)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The way along the ground at `azimuth`, a yaw from −Z towards +X.
fn way(azimuth: f32) -> Vec2 {
    Vec2::new(azimuth.sin(), -azimuth.cos())
}

/// How high the foothills' ridge rises at `azimuth`, rolling between 2 and 6 m, low enough that the ranges show above.
fn ridge(azimuth: f32) -> f32 {
    2.0 + 1.9 * (0.5 + 0.5 * (3.0 * azimuth + 0.7).sin())
        + 1.4 * (0.5 + 0.5 * (5.0 * azimuth + 2.1).sin())
        + 0.7 * (0.5 + 0.5 * (11.0 * azimuth + 0.3).sin())
}

/// How far `point` is along the waterfall's way from [`BACKDROP_CENTRE`], and how far across it, on the ground.
fn by_waterfall(point: Vec2) -> (f32, f32) {
    let (along, across) = on_the_fall(point);
    (along, across.abs())
}

/// The waterfall's way from [`BACKDROP_CENTRE`], on the ground, and the way across it to the right, as one faces the
/// fall from the yard.
pub fn fall_axes() -> (Vec2, Vec2) {
    let outward = way(layout::WATERFALL_AZIMUTH);
    (outward, Vec2::new(-outward.y, outward.x))
}

/// How far `point` is along the waterfall's way from [`BACKDROP_CENTRE`], and how far across it, to the right as one
/// faces the fall from the yard, on the ground.
pub fn on_the_fall(point: Vec2) -> (f32, f32) {
    let offset = point - BACKDROP_CENTRE;
    let (outward, right) = fall_axes();
    (offset.dot(outward), offset.dot(right))
}

/// How high the foothills' ground lies beneath the terrace's floor, a little below it, so that the floor is drawn on it
/// without flickering: the mountain's at the top of the falls.
pub const TERRACE_GROUND: f32 = layout::TERRACE - 0.05;

/// How far the mountain behind the waterfall rises at `point`, from none to all of it: across the waterfall's way, wider
/// to its right, where the terrace is, from the cliff's foot to its lip, and out to where it falls away into the ridge.
fn mountain(point: Vec2) -> f32 {
    let (along, across) = on_the_fall(point);
    let radius = point.distance(BACKDROP_CENTRE);
    let wide = (1.0 - smoothstep(5.0, 12.0, -across)) * (1.0 - smoothstep(12.0, 17.0, across));
    wide * (1.0 - smoothstep(33.0, 36.5, radius))
        * smoothstep(layout::CLIFF[0], layout::CLIFF[1], along)
}

/// How far `point` is from the stream that runs from the upper pool's middle to the waterfall's lip, on the ground, and
/// how far along it from the lip, from none to all of it.
fn from_stream(point: Vec2) -> (f32, f32) {
    let lip = Vec2::new(layout::WATERFALL_LIP.x, layout::WATERFALL_LIP.z);
    let head = Vec2::new(layout::UPPER_POOL.x, layout::UPPER_POOL.z);
    let along = head - lip;
    let t = ((point - lip).dot(along) / along.length_squared()).clamp(0.0, 1.0);
    (point.distance(lip + along * t), t)
}

/// How deep the stream's bed lies below its water, so that it is the waterfall's lip where it leaves; and how far from
/// its middle its bed reaches and its banks rise to the terrace's ground, at the lip and at the upper pool: its banks
/// rising through its water's level at the edges of its water and the waterfall's top, 0.6 m from its middle, at the
/// lip, and before them at the pool, so that the water's edges are beneath them.
const STREAM_DEPTH: f32 = 0.08;
const STREAM_BANKS: [[f32; 2]; 2] = [[0.42, 0.88], [0.3, 0.7]];

/// How far above its water every bank's top stands at least, so that the water lies in a channel below the land, the
/// trail and the yard, rather than above them.
pub const LIP: f32 = 0.4;
/// How much higher than [`LIP`] the river's banks rise in places along it, so that they vary.
pub const RISE: f32 = 0.3;

/// How far from the river's middle its bed rises towards its banks, where the channel reaches its bank's top, beyond
/// where its water's surface reaches beneath it, how far the bank's top holds there before the valley eases back into
/// the land, and where it has.
pub const BANKS: [f32; 4] = [
    0.9,
    RIVER_HALF_WIDTH + SPREAD - 0.1,
    RIVER_HALF_WIDTH + SPREAD + 0.5,
    RIVER_HALF_WIDTH + SPREAD + 1.9,
];

/// The same for the pool, from its middle: its basin's floor, its shore rising to its rim, the rim held beyond its
/// water and easing back into the land.
pub const RIM: [f32; 4] = [
    POOL_RADIUS - 1.4,
    POOL_RADIUS + 0.4,
    POOL_RADIUS + 1.4,
    POOL_RADIUS + 3.0,
];

/// How deep the river's bed and the pool's basin lie below their water.
const RIVER_DEPTH: f32 = 0.15;
const POOL_DEPTH: f32 = 0.3;

/// How high above its water the river's banks stand by its middle at `point`: from [`LIP`] to [`RISE`] more, rolling
/// along it, the same either side.
pub fn bank(point: Vec3) -> f32 {
    LIP + RISE * (0.5 + 0.5 * (0.45 * point.x + 0.3 * point.z).sin())
}

/// `height`, sunk for water at `level`, `distance` from its middle, as `profile` gives the shape of its banks, `depth`
/// how deep its bed lies and `lip` how far above the water their tops stand: the valley's floor lowered to the banks'
/// tops near the water, widening where the land is higher, so that its sides are not cliffs, and the channel cut from
/// there down to the bed. It never raises the land.
fn sunk(
    height: f32,
    level: f32,
    distance: f32,
    (profile, depth, lip): ([f32; 4], f32, f32),
) -> f32 {
    let top = level + lip;
    let wide = profile[2] + 2.0 + (height - top).max(0.0) * 1.2;
    let valley = top + (height - top) * smoothstep(profile[1], wide, distance);
    let height = height.min(valley);
    let bed = level - depth;
    height.min(bed + (height - bed) * smoothstep(profile[0], profile[1], distance))
}

/// `height`, raised to a bank by water at `level`, `distance` from its middle, as `profile` gives the bank's shape and
/// `depth` how deep its bed lies, so far as `weight` takes it: from its bed to [`LIP`] above the water, held, then eased
/// back into the land, which it never lowers. Where the land is already above that, as it is about the farmyard's water,
/// nothing changes.
fn banked(
    height: f32,
    level: f32,
    distance: f32,
    (profile, depth): ([f32; 4], f32),
    weight: f32,
) -> f32 {
    let bank = level - depth + (depth + LIP) * smoothstep(profile[0], profile[1], distance);
    let hold = 1.0 - smoothstep(profile[2], profile[3], distance);
    height + (bank - height).max(0.0) * hold * weight
}

/// How high the ground is at `point` beyond the fence, smoothly, as the foothills' ground is built about it: level with
/// the meadow within [`RINGS`]' first, rising across the pasture to the foothills, sunk for the pool and the river, whose
/// water lies in a channel below the land either side, the river running downhill in it, and raised for the waterfall's
/// cliff and the spur behind it.
pub fn ground(point: Vec2) -> f32 {
    let offset = point - BACKDROP_CENTRE;
    let radius = offset.length();
    if radius <= RINGS[0] {
        return 0.0;
    }
    let azimuth = offset.x.atan2(-offset.y);
    let mut height = APRON * smoothstep(12.0, 22.0, radius)
        + ridge(azimuth) * smoothstep(21.0, 34.0, radius).powf(1.5);
    // The pool's hollow and its basin, and the river's valley and channel, below the land either side.
    let pool = point.distance(Vec2::new(POOL.x, POOL.z));
    height = sunk(height, POOL.y, pool, (RIM, POOL_DEPTH, LIP));
    let (nearest, distance) = layout::nearest_river(point);
    height = sunk(
        height,
        nearest.y,
        distance,
        (BANKS, RIVER_DEPTH, bank(nearest)),
    );
    // The banks, a lip above the water, wherever the land would lie lower: the pool's rim, but where the river leaves
    // it, and the river's, but within the pool's own.
    let outflow = smoothstep(BANKS[0], BANKS[1], distance);
    height = banked(height, POOL.y, pool, (RIM, POOL_DEPTH), outflow);
    let beyond = smoothstep(POOL_RADIUS + 0.6, RIM[2], pool);
    height = banked(height, nearest.y, distance, (BANKS, RIVER_DEPTH), beyond);
    // The mountain behind the waterfall: its cliff, from its foot to its lip, the terrace's ground at the top of the
    // falls, the upper cliff behind it and the knoll above that, falling away beyond into the ridge.
    let (_, across) = on_the_fall(point);
    let upper = smoothstep(layout::UPPER_CLIFF[0], layout::UPPER_CLIFF[1], radius)
        * (1.0 - smoothstep(4.0, 8.0, -across))
        * (1.0 - smoothstep(11.0, 14.0, across));
    let level = TERRACE_GROUND + (layout::UPPER_TOP - TERRACE_GROUND) * upper;
    let height = height + (level - height).max(0.0) * mountain(point);
    // The upper pool's basin, and the stream's channel from it to the waterfall's lip, cut into the terrace's ground,
    // whose banks stand as far above their water as the river's do, and at the lip, its bed the cliff's top, so that the
    // waterfall falls as it did.
    let cut = |height: f32, depth: f32, distance: f32, [bed, bank]: [f32; 2]| {
        let floor = layout::UPPER_POOL.y - depth;
        height.min(floor + (height - floor) * smoothstep(bed, bank, distance))
    };
    let pool = point.distance(Vec2::new(layout::UPPER_POOL.x, layout::UPPER_POOL.z));
    let shore = layout::UPPER_POOL_RADIUS;
    let height = cut(height, POOL_DEPTH, pool, [shore - 0.5, shore + 0.2]);
    let (distance, along) = from_stream(point);
    let [lip, head] = STREAM_BANKS;
    let banks = [0, 1].map(|edge| lip[edge] + (head[edge] - lip[edge]) * along);
    cut(height, STREAM_DEPTH, distance, banks)
}

/// A flat polygon through `corners`, lit from the side `outward` points to, whichever way they wind.
pub(crate) fn facing(shapes: &mut Shapes, corners: &[Vec3], outward: Vec3, color: Vec3) {
    let normal = (corners[1] - corners[0]).cross(corners[2] - corners[0]);
    if normal.dot(outward) < 0.0 {
        let reversed: Vec<Vec3> = corners.iter().rev().copied().collect();
        shapes.polygon(&reversed, color);
    } else {
        shapes.polygon(corners, color);
    }
}

/// The foothills' cells drawn finely, about the stream and the upper pool, which the rings are too coarse to draw: those
/// from the waterfall's lip, [`layout::CLIFF`]'s top, to the upper cliff's, [`layout::UPPER_CLIFF`]'s, by their rings
/// in [`RINGS`], and from [`FINE_SEGMENTS`] segments either side of the waterfall's way. Each is cut into
/// `FINE_ROWS` rows, by its ring, and `FINE_COLUMNS` columns, whose corners are on [`ground`]; the cells beside
/// them take the corners along the edge they share, so that the ground is one surface, closed along every seam.
pub const FINE_RINGS: std::ops::Range<usize> = 8..13;
/// How many segments either side of the waterfall's way are drawn finely.
pub const FINE_SEGMENTS: usize = 2;
/// How many rows each ring of [`FINE_RINGS`] is cut into, about 30 cm each, and how many columns each segment, about
/// 25 to 30 cm each.
const FINE_ROWS: [usize; 5] = [5, 5, 7, 4, 4];
const FINE_COLUMNS: usize = 6;

/// The segment of [`SEGMENTS`] the waterfall's way runs along, round the rings.
fn waterfall_segment() -> usize {
    (layout::WATERFALL_AZIMUTH / TAU * SEGMENTS as f32)
        .round()
        .rem_euclid(SEGMENTS as f32) as usize
}

/// Whether the foothills' cell from `ring` and `segment` is drawn finely.
pub fn fine(ring: usize, segment: usize) -> bool {
    let from = (waterfall_segment() + SEGMENTS - FINE_SEGMENTS) % SEGMENTS;
    FINE_RINGS.contains(&ring) && (segment + SEGMENTS - from) % SEGMENTS < FINE_SEGMENTS * 2
}

/// How many rows the fine cells from `ring` are cut into.
fn fine_rows(ring: usize) -> usize {
    FINE_ROWS[ring - FINE_RINGS.start]
}

/// The corner `row` of the way from ring `ring` to the next and `column` of the way round from segment `segment` to the
/// next, of the fine cells: a corner of the rings' own where it is one, and otherwise on [`ground`], across its cell
/// from its corners, along their edges as they run straight. Found the same, to the bit, from either cell beside it.
fn fine_corner(ring: usize, row: usize, segment: usize, column: usize) -> Vec3 {
    let (ring, row) = if row > 0 && row == fine_rows(ring) {
        (ring + 1, 0)
    } else {
        (ring, row)
    };
    let (segment, column) = if column == FINE_COLUMNS {
        ((segment + 1) % SEGMENTS, 0)
    } else {
        (segment, column)
    };
    let next = (segment + 1) % SEGMENTS;
    let round = column as f32 / FINE_COLUMNS as f32;
    let point = match (row, column) {
        (0, 0) => return vertex(ring, segment),
        (0, _) => vertex(ring, segment).lerp(vertex(ring, next), round),
        _ => {
            let out = row as f32 / fine_rows(ring) as f32;
            let near = vertex(ring, segment).lerp(vertex(ring, next), round);
            let far = vertex(ring + 1, segment).lerp(vertex(ring + 1, next), round);
            if column == 0 {
                vertex(ring, segment).lerp(vertex(ring + 1, segment), out)
            } else {
                near.lerp(far, out)
            }
        }
    };
    Vec3::new(point.x, ground(Vec2::new(point.x, point.z)), point.z)
}

/// The triangles the foothills' cell from `ring` and `segment` is drawn in: its rows and columns, if it is drawn finely;
/// one beside those, taking in the fine corners along the edge it shares with them ([`zipped`]); and any other, the two of
/// [`cell`].
fn cell_triangles(ring: usize, segment: usize) -> Vec<[Vec3; 3]> {
    let next = (segment + 1) % SEGMENTS;
    if fine(ring, segment) {
        let rows = fine_rows(ring);
        return (0..rows)
            .flat_map(|row| (0..FINE_COLUMNS).map(move |column| (row, column)))
            .flat_map(|(row, column)| {
                let corner = |row: usize, column: usize| fine_corner(ring, row, segment, column);
                let [a, b, c, d] = [
                    corner(row, column),
                    corner(row, column + 1),
                    corner(row + 1, column + 1),
                    corner(row + 1, column),
                ];
                if (row + column) % 2 == 0 {
                    [[a, d, c], [a, c, b]]
                } else {
                    [[a, d, b], [b, d, c]]
                }
            })
            .collect();
    }
    let [a, b, c, d] = [
        vertex(ring, segment),
        vertex(ring, next),
        vertex(ring + 1, next),
        vertex(ring + 1, segment),
    ];
    let previous = (segment + SEGMENTS - 1) % SEGMENTS;
    // Its corners in order round it, from the start of the edge it shares with fine cells, along that edge's fine
    // corners, then round the rest.
    let shared: Option<Vec<Vec3>> = if ring > 0 && fine(ring - 1, segment) {
        let along = (1..FINE_COLUMNS).map(|column| fine_corner(ring, 0, segment, column));
        Some([a].into_iter().chain(along).chain([b, c, d]).collect())
    } else if fine(ring, next) {
        let up = (1..fine_rows(ring)).map(|row| fine_corner(ring, row, next, 0));
        Some([b].into_iter().chain(up).chain([c, d, a]).collect())
    } else if ring + 1 < RINGS.len() - 1 && fine(ring + 1, segment) {
        let back = (1..FINE_COLUMNS)
            .rev()
            .map(|column| fine_corner(ring + 1, 0, segment, column));
        Some([c].into_iter().chain(back).chain([d, a, b]).collect())
    } else if fine(ring, previous) {
        let down = (1..fine_rows(ring))
            .rev()
            .map(|row| fine_corner(ring, row, segment, 0));
        Some([d].into_iter().chain(down).chain([a, b, c]).collect())
    } else {
        None
    };
    match shared {
        Some(corners) => zipped(&corners),
        None => cell((ring, segment), [a, b, c, d]).to_vec(),
    }
}

/// The triangles of a cell beside the fine cells, its `corners` in order round it: the shared edge's, from its start to
/// its end, then the cell's other two. Its first half is fanned from the last corner, which is beside the shared edge's
/// start, and its second from the one before, which is beside its end, a triangle between them bridging the cell, so
/// that no corner takes more than half the edge's pieces, as a fan from one corner would, in long thin slivers.
fn zipped(corners: &[Vec3]) -> Vec<[Vec3; 3]> {
    let count = corners.len();
    let (near, far) = (corners[count - 1], corners[count - 2]);
    let edge = &corners[..count - 2];
    let middle = (edge.len() - 1) / 2;
    let mut triangles: Vec<[Vec3; 3]> = edge[..=middle]
        .windows(2)
        .map(|pair| [near, pair[0], pair[1]])
        .collect();
    triangles.push([near, edge[middle], far]);
    triangles.extend(
        edge[middle..]
            .windows(2)
            .map(|pair| [far, pair[0], pair[1]]),
    );
    triangles
}

/// The foothills' triangles, each cell's in turn, as drawn, found once.
pub fn foothills_triangles() -> &'static [Vec<[Vec3; 3]>] {
    static CELLS: std::sync::OnceLock<Vec<Vec<[Vec3; 3]>>> = std::sync::OnceLock::new();
    CELLS.get_or_init(|| {
        (0..RINGS.len() - 1)
            .flat_map(|ring| (0..SEGMENTS).map(move |segment| cell_triangles(ring, segment)))
            .collect()
    })
}

/// How high the triangle `corners` is at `point`, seen from above, if it is over it.
fn on([a, b, c]: &[Vec3; 3], point: Vec2) -> Option<f32> {
    let flat = |corner: &Vec3| Vec2::new(corner.x, corner.z);
    let (a2, b2, c2) = (flat(a), flat(b), flat(c));
    let area = (b2 - a2).perp_dot(c2 - a2);
    let u = (b2 - point).perp_dot(c2 - point) / area;
    let v = (c2 - point).perp_dot(a2 - point) / area;
    let w = 1.0 - u - v;
    (u >= -1e-5 && v >= -1e-5 && w >= -1e-5).then_some(a.y * u + b.y * v + c.y * w)
}

/// The point of the foothills' ground on ring `ring` of [`RINGS`], at segment `segment` of [`SEGMENTS`] round it: on
/// [`ground`], level with the meadow, [`MEADOW`], on the first ring, and a little rough further out, but along the river
/// and on the mountain behind the waterfall.
fn vertex(ring: usize, segment: usize) -> Vec3 {
    let radius = RINGS[ring];
    let point = BACKDROP_CENTRE + way(segment as f32 / SEGMENTS as f32 * TAU) * radius;
    let rough = smoothstep(25.0, 30.0, radius)
        * smoothstep(3.0, 6.0, layout::nearest_river(point).1)
        * (1.0 - mountain(point));
    let jitter = (scatter((ring * SEGMENTS + segment) as u32 + 7000) - 0.5) * 0.6;
    let height = if ring == 0 {
        MEADOW
    } else {
        ground(point) + jitter * rough
    };
    Vec3::new(point.x, height, point.y)
}

/// The two triangles the cell of the foothills' ground from `ring` and `segment` is cut into, from its corners on that
/// ring, `a` and `b`, and the next, `c` and `d`, the diagonal alternating from cell to cell, but for the cells from the
/// upper cliff's foot, which keep the way they were cut before its ring was added, so that the river's channel beyond
/// is drawn as it was.
fn cell((ring, segment): (usize, usize), [a, b, c, d]: [Vec3; 4]) -> [[Vec3; 3]; 2] {
    let ring = ring - usize::from(RINGS[ring] >= layout::UPPER_CLIFF[0]);
    if (ring + segment) % 2 == 0 {
        [[a, d, c], [a, c, b]]
    } else {
        [[a, d, b], [b, d, c]]
    }
}

/// The meadow's edge, the corners of the foothills' first ring, in order round it, at [`MEADOW`]'s height: the yard's
/// meadow is the flat polygon through them.
pub fn meadow() -> Vec<Vec3> {
    (0..SEGMENTS).map(|segment| vertex(0, segment)).collect()
}

/// How high the foothills' ground is drawn at `point`, from its triangle there, which follows [`ground`] only at its
/// corners, finely about the stream and the upper pool; `None` within the meadow inside the first ring, or beyond the
/// last.
pub fn drawn_ground(point: Vec2) -> Option<f32> {
    let offset = point - BACKDROP_CENTRE;
    let radius = offset.length();
    let ring = RINGS.windows(2).position(|pair| radius <= pair[1])?;
    let azimuth = offset.x.atan2(-offset.y).rem_euclid(TAU);
    let segment = (azimuth / TAU * SEGMENTS as f32) as usize;
    // Its cell, or, since a cell's edges are chords inside its rings, one beside it.
    let cells = foothills_triangles();
    let rings = ring.saturating_sub(1)..=(ring + 1).min(RINGS.len() - 2);
    rings
        .flat_map(|ring| [SEGMENTS - 1, 0, 1].map(move |step| (ring, (segment + step) % SEGMENTS)))
        .flat_map(|(ring, segment)| &cells[ring * SEGMENTS + segment])
        .find_map(|corners| on(corners, point))
}

/// The foothills' ground as it is drawn: each of its cells' triangles, ring by ring and segment by segment, with the
/// ring and segment of its cell, finely about the stream and the upper pool ([`foothills_triangles`]).
pub fn ground_triangles() -> Vec<((usize, usize), [Vec3; 3])> {
    foothills_triangles()
        .iter()
        .enumerate()
        .flat_map(|(index, triangles)| {
            triangles
                .iter()
                .map(move |corners| ((index / SEGMENTS, index % SEGMENTS), *corners))
        })
        .collect()
}

/// The foothills' ground, a ring of flat triangles from just beyond the fence to the ridge: pasture near the yard,
/// native bush on the foothills, grey shingle along the river and about the pool, and rock where it is steep, as at
/// the waterfall's cliff; about the stream and the upper pool, stone in their channel and basin and bush beyond, each
/// face a shade of its own; and, where the beach is, its own triangles cut along the beach's outline, the part within
/// it the beach's shingle ([`crate::beach::shingle`]).
fn foothills(shapes: &mut Shapes) {
    let beach = crate::beach::outline();
    let mut index = 0;
    for (cell, triangles) in foothills_triangles().iter().enumerate() {
        let (ring, segment) = (cell / SEGMENTS, cell % SEGMENTS);
        let fine = fine(ring, segment);
        for (part, corners) in triangles.iter().enumerate() {
            index += 1;
            let middle = (corners[0] + corners[1] + corners[2]) / 3.0;
            let normal = (corners[1] - corners[0])
                .cross(corners[2] - corners[0])
                .normalize();
            let at = Vec2::new(middle.x, middle.z);
            let radius = at.distance(BACKDROP_CENTRE);
            let wet = layout::nearest_river(at).1 < RIVER_HALF_WIDTH + 2.4
                || at.distance(Vec2::new(POOL.x, POOL.z)) < POOL_RADIUS + 0.8;
            // About the stream and the upper pool, told by the middle of the cell of rows and columns it is half of, so
            // that the stone's edge runs along the cells' edges rather than to and fro across them.
            let stream = fine && {
                let pair = &triangles[part - part % 2..part - part % 2 + 2];
                let quad = pair.iter().flatten().sum::<Vec3>() / 6.0;
                let upper = Vec2::new(quad.x, quad.z)
                    .distance(Vec2::new(layout::UPPER_POOL.x, layout::UPPER_POOL.z));
                quad.y < TERRACE_GROUND - 0.06 || upper < layout::UPPER_POOL_RADIUS + 0.25
            };
            let shade = 0.92 + scatter((cell * 64 + part) as u32 + 19_000) * 0.16;
            let color = if normal.y.abs() < 0.6 {
                colors::ROCK
            } else if stream {
                colors::SHINGLE * shade
            } else if fine {
                colors::BUSH * shade
            } else if wet {
                colors::SHINGLE
            } else if radius < 19.0 + scatter(((ring * SEGMENTS + segment) * 2) as u32) * 4.0 {
                palette::GRASS
            } else {
                colors::BUSH
            };
            let (within, beyond) = crate::shapes::cut(corners, &beach);
            for part in beyond {
                facing(shapes, &part, Vec3::Y, color);
            }
            if let Some(part) = within {
                facing(shapes, &part, Vec3::Y, crate::beach::shingle(index as u32));
            }
        }
    }
}

/// The materials of the backdrop's models: the yard's.
pub(crate) fn built(name: &str, shapes: Shapes) -> Model {
    Model::built(
        vec![Node::new(name, Transform::IDENTITY).with_mesh(0)],
        vec![0],
        vec![Mesh::new(name, vec![shapes.primitive(PLAIN)])],
        props::materials(),
        &Limits::default(),
    )
    .expect("the backdrop is built within the limits")
}

/// The foothills' ground, as a model of one node whose mesh draws it all, in the yard's coordinates.
pub fn model() -> Model {
    let mut shapes = Shapes::new();
    foothills(&mut shapes);
    built("foothills", shapes)
}

/// The kinds of plant in the bush, each a model of its own, instanced where [`plants`] says, but the tree ferns, drawn as
/// the ferns near the trail are.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    /// A tree fern, ponga or mamaku, drawn with the ferns' meshes and their shader, which sways them
    /// ([`crate::ferns::bush`]), as large as its size says.
    Fern,
    /// A cabbage tree, tī kōuka: a pale trunk branching into spiky tufts of leaves.
    CabbageTree,
    /// A podocarp, rimu or kahikatea: a tall straight trunk and a narrow bronze-green crown above the canopy.
    Podocarp,
    /// Flax, harakeke: a fan of long sword-like leaves, with a dark flower stalk.
    Flax,
    /// The bush's canopy: a low mound of broadleaf trees, which clads the foothills.
    Canopy,
}

impl Kind {
    /// Every kind, in the order the scenery draws them.
    pub const ALL: [Kind; 5] = [
        Kind::Canopy,
        Kind::Podocarp,
        Kind::Fern,
        Kind::CabbageTree,
        Kind::Flax,
    ];

    /// How far it reaches from its middle on the ground, and how tall it stands, at its own size.
    pub fn size(self) -> (f32, f32) {
        match self {
            Kind::Fern => (2.3, 3.9),
            Kind::CabbageTree => (1.8, 4.6),
            Kind::Podocarp => (2.2, 12.2),
            Kind::Flax => (1.0, 2.2),
            Kind::Canopy => (2.2, 2.6),
        }
    }

    /// Its model, built in code, standing on its origin, its foot sunk a little so that it stands on a slope; or none
    /// for a tree fern, which the ferns' meshes draw.
    pub fn model(self) -> Option<Model> {
        let mut shapes = Shapes::new();
        let name = match self {
            Kind::Fern => return None,
            Kind::CabbageTree => {
                cabbage_tree(&mut shapes);
                "cabbage tree"
            }
            Kind::Podocarp => {
                podocarp(&mut shapes);
                "podocarp"
            }
            Kind::Flax => {
                flax(&mut shapes);
                "flax"
            }
            Kind::Canopy => {
                canopy(&mut shapes);
                "canopy"
            }
        };
        Some(built(name, shapes))
    }
}

/// How far below the ground every plant's foot reaches, so that none floats on a slope.
const SUNK: f32 = 0.4;

/// A blade from `base`, `out` along the ground and `rise` up, `width` wide at `base` and tapering to its tip, in
/// `sections` flat pieces, its tip drooping by `droop`.
fn blade(
    shapes: &mut Shapes,
    base: Vec3,
    yaw: f32,
    (out, rise, droop): (f32, f32, f32),
    widths: &[f32],
    color: Vec3,
) {
    let outward = Vec3::new(yaw.sin(), 0.0, yaw.cos());
    let side = Vec3::new(outward.z, 0.0, -outward.x);
    let sections = widths.len();
    let point =
        |t: f32| base + outward * out * t + Vec3::Y * (rise * (t * PI * 0.5).sin() - droop * t * t);
    let mut previous = [point(0.0) + side * widths[0], point(0.0) - side * widths[0]];
    let up = Vec3::Y + outward * 0.2;
    // Each section to the next width, and the last to the tip.
    for (index, width) in widths[1..].iter().map(Some).chain([None]).enumerate() {
        let middle = point((index + 1) as f32 / sections as f32);
        match width {
            Some(width) => {
                let next = [middle + side * *width, middle - side * *width];
                facing(
                    shapes,
                    &[previous[0], previous[1], next[1], next[0]],
                    up,
                    color,
                );
                previous = next;
            }
            None => facing(shapes, &[previous[0], previous[1], middle], up, color),
        }
    }
}

fn cabbage_tree(shapes: &mut Shapes) {
    shapes.cylinder(
        Vec3::new(0.0, -SUNK, 0.0),
        0.11,
        2.3 + SUNK,
        5,
        colors::CABBAGE_BARK,
    );
    let fork = Vec3::new(0.0, 2.2, 0.0);
    for branch in 0..3 {
        let yaw = (branch as f32 + scatter(branch + 400) * 0.4) / 3.0 * TAU;
        let out = 0.5 + scatter(branch + 410) * 0.35;
        let tip = fork
            + Vec3::new(
                yaw.sin() * out,
                1.0 + scatter(branch + 420) * 0.5,
                yaw.cos() * out,
            );
        shapes.beam(fork, tip, 0.09, colors::CABBAGE_BARK);
        // A tuft: a ball of leaves, and the longest spiking out of it.
        clump(
            shapes,
            (tip, 0.42, 0.6),
            false,
            440 + branch * 30,
            &[colors::CABBAGE, colors::CABBAGE * 0.8],
        );
        for leaf in 0..8 {
            let seed = branch * 20 + leaf + 430;
            let yaw = (leaf as f32 + scatter(seed) * 0.6) / 8.0 * TAU;
            // Most point up and out; the lowest droop, as old leaves do.
            let pitch = if leaf % 4 == 3 {
                -0.5
            } else {
                0.35 + scatter(seed + 200) * 0.9
            };
            let along = Vec3::new(
                yaw.sin() * pitch.cos(),
                pitch.sin(),
                yaw.cos() * pitch.cos(),
            );
            let side = Vec3::new(yaw.cos(), 0.0, -yaw.sin()) * 0.08;
            facing(
                shapes,
                &[tip + side, tip - side, tip + along * 0.8],
                Vec3::Y,
                colors::CABBAGE,
            );
        }
    }
}

/// An irregular low-poly clump about `centre`, `radius` wide and `height` tall: two rings of six corners, each a
/// little nearer or further, the upper turned half a corner from the lower, with a point above, and below, unless it
/// sits on the ground, a point below, so that it reads as a rounded mass of leaves.
pub(crate) fn clump(
    shapes: &mut Shapes,
    (centre, radius, height): (Vec3, f32, f32),
    grounded: bool,
    seed: u32,
    colors: &[Vec3],
) {
    let sides = 6u32;
    let ring = |level: f32, turn: f32, wide: f32, offset: u32| -> Vec<Vec3> {
        (0..sides)
            .map(|side| {
                let yaw = (side as f32 + turn + (scatter(seed + side + offset) - 0.5) * 0.4)
                    / sides as f32
                    * TAU;
                let reach = radius * wide * (0.82 + scatter(seed + side + offset + 50) * 0.36);
                centre + Vec3::new(yaw.sin() * reach, height * level, yaw.cos() * reach)
            })
            .collect()
    };
    let (lower, upper) = if grounded {
        (ring(-0.5, 0.0, 1.0, 0), ring(0.05, 0.5, 0.8, 100))
    } else {
        (ring(-0.2, 0.0, 0.95, 0), ring(0.18, 0.5, 0.8, 100))
    };
    let top = centre + Vec3::new(0.0, height * 0.5, 0.0);
    let bottom = centre - Vec3::new(0.0, height * 0.5, 0.0);
    let sides = sides as usize;
    for side in 0..sides {
        let next = (side + 1) % sides;
        let color = colors[side % colors.len()];
        let outward = |corners: &[Vec3]| {
            corners.iter().copied().sum::<Vec3>() / corners.len() as f32 - centre
        };
        let band = [lower[side], lower[next], upper[side]];
        facing(shapes, &band, outward(&band), color);
        let band = [lower[next], upper[next], upper[side]];
        facing(shapes, &band, outward(&band), color * 0.92);
        let cap = [upper[side], upper[next], top];
        facing(shapes, &cap, outward(&cap), color * 1.08);
        if !grounded {
            let under = [lower[next], lower[side], bottom];
            facing(shapes, &under, outward(&under), color * 0.8);
        }
    }
}

fn podocarp(shapes: &mut Shapes) {
    shapes.cylinder(
        Vec3::new(0.0, -SUNK, 0.0),
        0.26,
        9.0 + SUNK,
        6,
        colors::PODOCARP_BARK,
    );
    let crown = [
        colors::PODOCARP,
        colors::PODOCARP * 1.25,
        Vec3::new(0.08, 0.085, 0.04),
    ];
    for (index, (y, radius, height)) in [
        (6.0, 1.4, 2.8),
        (7.5, 1.65, 2.8),
        (9.0, 1.3, 2.6),
        (10.6, 0.9, 2.2),
    ]
    .into_iter()
    .enumerate()
    {
        let lean = Vec3::new(
            scatter(index as u32 + 500) - 0.5,
            0.0,
            scatter(index as u32 + 510) - 0.5,
        ) * 1.1;
        clump(
            shapes,
            (Vec3::new(0.0, y, 0.0) + lean, radius, height),
            false,
            520 + index as u32 * 100,
            &crown,
        );
    }
}

fn flax(shapes: &mut Shapes) {
    for leaf in 0..9 {
        let yaw = (leaf as f32 + (scatter(leaf + 600) - 0.5) * 0.6) / 9.0 * TAU;
        let out = 0.3 + scatter(leaf + 610) * 0.55;
        let rise = 1.0 + scatter(leaf + 620) * 0.6;
        blade(
            shapes,
            Vec3::ZERO,
            yaw,
            (out, rise, out * 0.4),
            &[0.05, 0.035],
            colors::FLAX * (0.85 + scatter(leaf + 630) * 0.3),
        );
    }
    shapes.beam(
        Vec3::new(0.0, -0.1, 0.0),
        Vec3::new(0.12, 2.15, 0.05),
        0.03,
        colors::STALK,
    );
}

fn canopy(shapes: &mut Shapes) {
    clump(
        shapes,
        (Vec3::new(0.0, 1.0, 0.0), 1.8, 2.8),
        true,
        700,
        &[colors::BUSH, colors::BUSH_DARK, colors::BUSH_LIGHT],
    );
}

/// A plant of the bush: its kind, where its foot stands, how it is turned about the vertical and how large it is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Plant {
    pub kind: Kind,
    pub at: Vec3,
    pub turn: f32,
    pub scale: f32,
}

impl Plant {
    /// Where its model is drawn.
    pub fn transform(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(
            Vec3::splat(self.scale),
            Quat::from_rotation_y(self.turn),
            self.at,
        )
    }

    /// How far it reaches on the ground, and how tall it stands.
    pub fn size(&self) -> (f32, f32) {
        let (reach, height) = self.kind.size();
        (reach * self.scale, height * self.scale)
    }

    /// The box it stands within, from its foot to its top.
    pub fn solid(&self) -> Solid {
        let (reach, height) = self.size();
        Solid::new(
            (self.at - Vec3::new(reach, SUNK * self.scale, reach)).to_array(),
            (self.at + Vec3::new(reach, height, reach)).to_array(),
        )
    }
}

/// The sightlines the backdrop keeps open: from the eyes where participants arrive, to the waterfall's lip, its middle
/// and the mist at its foot, and to the river's far bank behind the coop and to their right, since its water lies below
/// its banks; from the eyes of one standing in the workbench's curve, to the river's far bank beyond the yard's right
/// side; and from the trail, its lookout to the pool and the waterfall rock, and its riverbank to the river.
pub fn views() -> Vec<([f32; 3], [f32; 3])> {
    let eyes = |at: Vec3, towards: Vec3| Spot::facing(at.to_array(), towards.to_array()).eyes();
    let arrival = eyes(layout::ARRIVAL, layout::ARRIVAL_FACING);
    let workbench = eyes(
        Vec3::new(layout::WORKBENCH.x, 0.0, layout::WORKBENCH.y),
        layout::ARRIVAL,
    );
    let bank = |point: Vec3, from: [f32; 3]| layout::far_bank(point, Vec2::new(from[0], from[2]));
    let mut views: Vec<_> = [
        layout::WATERFALL_LIP,
        layout::WATERFALL,
        layout::WATERFALL_FOOT + Vec3::Y * 0.6,
        bank(RIVER[2], arrival),
        bank(RIVER[7], arrival),
    ]
    .into_iter()
    .map(|to| (arrival, to.to_array()))
    .collect();
    views.extend(
        RIVER[6..9]
            .iter()
            .map(|to| (workbench, bank(*to, workbench).to_array())),
    );
    let trail = |index: usize| {
        let (point, _, _) = crate::trail::frame(index);
        let at = Vec3::new(point.x, crate::trail::HEIGHT, point.y);
        eyes(at, at + Vec3::NEG_Z)
    };
    let [(_, _, riverbank, _), (_, _, lookout, _)] = crate::trail::VIEWPOINTS;
    views.extend(
        [POOL, layout::WATERFALL_ROCK + Vec3::Y * 0.3]
            .into_iter()
            .map(|to| (trail(lookout), to.to_array())),
    );
    views.extend(
        RIVER[6..8]
            .iter()
            .map(|to| (trail(riverbank), to.to_array())),
    );
    views
}

/// Whether something reaching `reach` about `point` on the ground, `height` tall, leaves the backdrop open: out of
/// the water, off the waterfall's cliff and out of the way to it, and clear of the sightlines [`views`] keeps open.
pub fn open(point: Vec2, reach: f32, height: f32) -> bool {
    let (along, across) = by_waterfall(point);
    // Nothing before the waterfall, nor on the knoll behind it, where it would stand against the peaks beyond, nor on
    // the terrace at the top of the falls or beside it.
    let cliff = across < 6.0 + reach && along > 15.0 && along < layout::UPPER_CLIFF[1] + 2.5;
    if cliff || crate::terrace::from_floor(point) < reach + 1.5 {
        return false;
    }
    let wet = layout::nearest_river(point).1 < RIVER_HALF_WIDTH + 1.6 + reach * 0.3
        || point.distance(Vec2::new(POOL.x, POOL.z)) < POOL_RADIUS + 0.6 + reach;
    if wet {
        return false;
    }
    let ground = ground(point);
    let solid = Solid::new(
        [point.x - reach, ground - SUNK, point.y - reach],
        [point.x + reach, ground + height, point.y + reach],
    );
    views().iter().all(|(from, to)| !solid.blocks(*from, *to))
}

/// Whether a plant reaching `reach` about `point` on the ground, `height` tall, may stand there: on the foothills'
/// ground, well beyond the fence, out of the way in through the gate, within the backdrop's 50 m bound, and leaving the
/// backdrop [`open`].
pub fn clear(point: Vec2, reach: f32, height: f32) -> bool {
    let radius = point.distance(BACKDROP_CENTRE);
    let gate = point.y > 4.0 && point.x.abs() < 6.0 + reach && radius < 26.0;
    let fence = layout::FENCE.grown(3.0 + reach).contains(point);
    radius >= RINGS[0] + 1.0 + reach
        && radius <= 36.0
        && !gate
        && !fence
        && open(point, reach, height)
}

/// Where the bush's plants of `kind` might stand, the `index`th of those tried: each kind where it grows, tree ferns
/// and flax by the water, cabbage trees on the pasture, podocarps and the canopy on the foothills, and every kind in the
/// bush's patches ([`layout::BUSH`]).
fn candidate(kind: Kind, index: u32) -> Vec2 {
    let seed = index * 7 + kind as u32 * 100_003;
    let (first, second, third) = (scatter(seed), scatter(seed + 1), scatter(seed + 2));
    let around = |from: f32, to: f32| {
        BACKDROP_CENTRE + way(first * TAU) * (from + (to - from) * second.sqrt())
    };
    let patch = || {
        let (middle, radius) = layout::BUSH[(third * layout::BUSH.len() as f32) as usize];
        middle + way(first * TAU) * radius * second.sqrt()
    };
    // Beside the water: the river's banks, either side, or the pool's shore.
    let water = |from: f32, to: f32| {
        let reach = (third * (RIVER.len() - 1) as f32 * 1.25) as usize;
        let side = if second < 0.5 { 1.0 } else { -1.0 };
        let distance = from + (to - from) * (second * 2.0).fract();
        if reach >= RIVER.len() - 1 {
            Vec2::new(POOL.x, POOL.z) + way(first * TAU) * (POOL_RADIUS + distance)
        } else {
            let (start, end) = (RIVER[reach], RIVER[reach + 1]);
            let along = Vec2::new(end.x - start.x, end.z - start.z);
            Vec2::new(start.x, start.z)
                + along * first
                + along.normalize().perp() * side * (RIVER_HALF_WIDTH + distance)
        }
    };
    let in_patch = scatter(seed + 3) < 0.3;
    match kind {
        _ if in_patch => patch(),
        Kind::Canopy => around(23.0, 35.5),
        Kind::Podocarp => around(23.0, 31.0),
        Kind::Fern => water(2.6, 8.0),
        Kind::Flax => water(2.2, 3.6),
        Kind::CabbageTree => around(13.5, 22.0),
    }
}

/// How many of each kind the bush has, and how large each grows, from the least to the most.
const COUNTS: [(Kind, usize, [f32; 2]); 5] = [
    (Kind::Canopy, 240, [0.8, 1.7]),
    (Kind::Podocarp, 16, [0.7, 0.95]),
    (Kind::Fern, 40, [0.75, 1.2]),
    (Kind::CabbageTree, 22, [0.8, 1.2]),
    (Kind::Flax, 40, [0.7, 1.2]),
];

/// The bush's plants, the same every time: of each kind, its number, each where [`clear`] lets it stand, apart from
/// those already placed and from the pines, turned and sized from its place.
pub fn plants() -> Vec<Plant> {
    let pines: Vec<(Vec2, f32)> = props::pines()
        .into_iter()
        .map(|(base, _)| (Vec2::new(base.x, base.z), 1.8))
        .collect();
    let mut placed: Vec<Plant> = Vec::new();
    for (kind, count, [least, most]) in COUNTS {
        let mut found = 0;
        let mut index = 0;
        while found < count && index < count as u32 * 200 {
            let point = candidate(kind, index);
            let seed = index * 13 + kind as u32 * 7_919;
            index += 1;
            let scale = least + (most - least) * scatter(seed + 5);
            let (reach, height) = kind.size();
            let (reach, height) = (reach * scale, height * scale);
            let apart = |other: Vec2, theirs: f32| point.distance(other) > (reach + theirs) * 0.45;
            if !clear(point, reach, height)
                || !pines.iter().all(|(pine, theirs)| apart(*pine, *theirs))
                || !placed
                    .iter()
                    .all(|plant| apart(Vec2::new(plant.at.x, plant.at.z), plant.size().0))
            {
                continue;
            }
            placed.push(Plant {
                kind,
                at: Vec3::new(point.x, ground(point), point.y),
                turn: scatter(seed + 6) * TAU,
                scale,
            });
            found += 1;
        }
    }
    placed
}

/// A river stone: where its foot sits, how it is turned about the vertical, and how large it is, across and up.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stone {
    pub at: Vec3,
    pub turn: f32,
    pub size: Vec2,
}

/// How high the stone's model reaches above its foot and below it, at a metre across.
pub const STONE_TOP: f32 = 0.42;
const STONE_BOTTOM: f32 = 0.15;

impl Stone {
    /// Where the stone's model is drawn.
    pub fn transform(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(
            Vec3::new(self.size.x, self.size.y, self.size.x),
            Quat::from_rotation_y(self.turn),
            self.at,
        )
    }

    /// The box it stands within.
    pub fn solid(&self) -> Solid {
        let (reach, up) = (self.size.x * 0.5, self.size.y);
        Solid::new(
            (self.at - Vec3::new(reach, STONE_BOTTOM * up, reach)).to_array(),
            (self.at + Vec3::new(reach, STONE_TOP * up, reach)).to_array(),
        )
    }
}

/// The corners of the model every river stone is drawn from, a metre across: its lower ring of five, its upper ring,
/// turned half a corner from it, and its crown.
fn stone_rings() -> (Vec<Vec3>, Vec<Vec3>, Vec3) {
    let ring = |level: f32, reach: f32, turn: f32, seed: u32| -> Vec<Vec3> {
        (0..5)
            .map(|side| {
                let yaw = (side as f32 + turn) / 5.0 * TAU;
                let reach = reach * (0.88 + scatter(seed + side) * 0.24);
                Vec3::new(yaw.sin() * reach, level, yaw.cos() * reach)
            })
            .collect()
    };
    (
        ring(-STONE_BOTTOM, 0.46, 0.0, 800),
        ring(0.24, 0.34, 0.5, 810),
        Vec3::new(0.04, STONE_TOP, -0.03),
    )
}

/// Every corner of the river stone's model, a metre across, its foot at its origin, of which it is the convex hull: what
/// a body knocked against it meets.
pub fn stone_corners() -> Vec<Vec3> {
    let (lower, upper, top) = stone_rings();
    lower.into_iter().chain(upper).chain([top]).collect()
}

/// The model every river stone is drawn from, a metre across, rounded and low, its foot at its origin, a little of it
/// below: two rings of five corners, the upper turned half a corner from the lower, and a crown.
pub fn stone() -> Model {
    let mut shapes = Shapes::new();
    let (lower, upper, top) = stone_rings();
    let middle = Vec3::Y * 0.05;
    for side in 0..5 {
        let next = (side + 1) % 5;
        let shade = 0.85 + scatter(side as u32 + 820) * 0.3;
        for (corners, light) in [
            ([lower[side], lower[next], upper[side]], 1.0),
            ([lower[next], upper[next], upper[side]], 0.93),
            ([upper[side], upper[next], top], 1.1),
        ] {
            let outward = corners.iter().copied().sum::<Vec3>() / 3.0 - middle;
            facing(
                &mut shapes,
                &corners,
                outward,
                colors::STONE * shade * light,
            );
        }
    }
    built("stone", shapes)
}

/// How many corners each ring of the waterfall rock has.
pub const ROCK_SIDES: usize = 8;

/// The rings of the waterfall rock, from its foot below the water to its top, each `(height, reach)` from its middle: at
/// the water, its widest; its shoulder; and the edge of its flat top, where one stands, at [`layout::WATERFALL_ROCK`]'s
/// height.
const ROCK_RINGS: [(f32, f32); 3] = [
    (POOL.y - 0.12, layout::ROCK_REACH),
    (layout::WATERFALL_ROCK.y - 0.12, layout::ROCK_REACH * 0.84),
    (layout::WATERFALL_ROCK.y, layout::ROCK_TOP),
];

/// The waterfall rock, as a model in the yard's coordinates: a low boulder, grey as the river's stones, rounded and
/// faceted, its top flat and paler, worn, where one stands, its foot below the water, which hides the rest of it. Every corner is
/// within [`layout::ROCK_REACH`] of its middle and its top exactly at its height, so that what is drawn is what the ray
/// points at and where one stands.
pub fn rock() -> Model {
    let mut shapes = Shapes::new();
    let middle = layout::WATERFALL_ROCK;
    let rings: Vec<Vec<Vec3>> = ROCK_RINGS
        .iter()
        .enumerate()
        .map(|(ring, (height, reach))| {
            (0..ROCK_SIDES)
                .map(|side| {
                    let yaw = (side as f32 + ring as f32 * 0.5) / ROCK_SIDES as f32 * TAU + 0.3;
                    let reach =
                        reach * (0.86 + scatter((ring * ROCK_SIDES + side) as u32 + 8600) * 0.14);
                    Vec3::new(
                        middle.x + yaw.sin() * reach,
                        *height,
                        middle.z + yaw.cos() * reach,
                    )
                })
                .collect()
        })
        .collect();
    let centre = Vec3::new(middle.x, 0.0, middle.z);
    for pair in rings.windows(2) {
        for side in 0..ROCK_SIDES {
            let next = (side + 1) % ROCK_SIDES;
            let shade = 0.9 + scatter(side as u32 + 8700) * 0.25;
            for corners in [
                [pair[0][side], pair[0][next], pair[1][side]],
                [pair[0][next], pair[1][next], pair[1][side]],
            ] {
                let outward = corners.iter().copied().sum::<Vec3>() / 3.0 - centre;
                let outward = Vec3::new(outward.x, 0.0, outward.z);
                facing(&mut shapes, &corners, outward, colors::STONE * 0.75 * shade);
            }
        }
    }
    // Its top, flat, from its middle, so that it is a floor however its edge turns.
    for side in 0..ROCK_SIDES {
        let corners = [middle, rings[2][side], rings[2][(side + 1) % ROCK_SIDES]];
        facing(&mut shapes, &corners, Vec3::Y, colors::STONE * 1.1);
    }
    built("rock", shapes)
}

/// How many stones lie along the river's banks, in its shallows and about the pool's shore.
pub const SHORE_STONES: u32 = 48;
pub const SHALLOW_STONES: u32 = 10;
pub const POOL_STONES: u32 = 16;

/// The river's stones, the same every time: grey and rounded, of every size, along its banks where they rise out of the
/// water, a few in its shallows, standing out of the water, and about the pool's shore, but where the river leaves it
/// and before the waterfall; each sitting on the ground as it is drawn, apart from the others and out of the views the
/// backdrop keeps open.
pub fn stones() -> Vec<Stone> {
    let line = layout::river_line(layout::RIVER_STEPS);
    let lengths: Vec<f32> = line
        .windows(2)
        .map(|pair| Vec2::new(pair[1].x - pair[0].x, pair[1].z - pair[0].z).length())
        .collect();
    let total: f32 = lengths.iter().sum();
    // The point of the river's middle `along` it, and the way across it there.
    let on_river = |mut along: f32| {
        for (index, length) in lengths.iter().enumerate() {
            if along <= *length || index == lengths.len() - 1 {
                let (from, to) = (line[index], line[index + 1]);
                let middle = from.lerp(to, (along / length).clamp(0.0, 1.0));
                let tangent = Vec2::new(to.x - from.x, to.z - from.z).normalize();
                return (middle, tangent.perp());
            }
            along -= length;
        }
        unreachable!("the river has reaches")
    };
    let pool = Vec2::new(POOL.x, POOL.z);
    let views = views();
    let mut placed: Vec<Stone> = Vec::new();
    let place = |placed: &mut Vec<Stone>, point: Vec2, foot: f32, size: Vec2, seed: u32| {
        let stone = Stone {
            at: Vec3::new(point.x, foot, point.y),
            turn: scatter(seed + 5) * TAU,
            size,
        };
        let apart = placed.iter().all(|other| {
            Vec2::new(other.at.x, other.at.z).distance(point) > (other.size.x + size.x) * 0.6
        });
        let solid = stone.solid();
        if apart && views.iter().all(|(from, to)| !solid.blocks(*from, *to)) {
            placed.push(stone);
        }
    };
    let shaped = |seed: u32, least: f32, most: f32| {
        let across = least + (most - least) * scatter(seed + 3).powi(2);
        Vec2::new(across, across * (0.75 + scatter(seed + 4) * 0.5))
    };
    // Along the banks, where they rise out of the water, beyond the pool.
    for index in 0..SHORE_STONES {
        let seed = 12_000 + index * 11;
        let (middle, across) =
            on_river((index as f32 + scatter(seed)) / SHORE_STONES as f32 * total);
        let side = if scatter(seed + 1) < 0.5 { 1.0 } else { -1.0 };
        let distance = BANKS[0] + 0.3 + scatter(seed + 2) * (BANKS[1] - BANKS[0] - 0.2);
        let point = Vec2::new(middle.x, middle.z) + across * side * distance;
        if point.distance(pool) < RIM[2] {
            continue;
        }
        // By the river, beyond the first ring; within it, the meadow is level.
        let foot = drawn_ground(point).unwrap_or(0.0);
        place(
            &mut placed,
            point,
            foot - 0.03,
            shaped(seed, 0.3, 0.85),
            seed,
        );
    }
    // In the shallows, standing out of the water, set into its bed.
    for index in 0..SHALLOW_STONES {
        let seed = 13_000 + index * 11;
        let (middle, across) =
            on_river((index as f32 + 0.3 + scatter(seed) * 0.4) / SHALLOW_STONES as f32 * total);
        let point = Vec2::new(middle.x, middle.z) + across * (scatter(seed + 2) - 0.5) * 1.6;
        if point.distance(pool) < RIM[2] {
            continue;
        }
        // Set into the bed as it is drawn, and standing out of the water, or not there at all.
        let size = shaped(seed, 0.45, 0.8);
        let bed = drawn_ground(point).unwrap_or(0.0);
        let foot = (middle.y - 0.08).min(bed + STONE_BOTTOM * 0.8 * size.y);
        if foot + STONE_TOP * size.y < middle.y + 0.06 {
            continue;
        }
        place(&mut placed, point, foot, size, seed);
    }
    // About the pool's shore, where it rises out of the water, but where the river leaves it and before the fall.
    for index in 0..POOL_STONES {
        let seed = 14_000 + index * 11;
        let yaw = (index as f32 + scatter(seed)) / POOL_STONES as f32 * TAU;
        let point =
            pool + Vec2::new(yaw.sin(), yaw.cos()) * (POOL_RADIUS - 0.5 + scatter(seed + 2) * 0.5);
        let (along, across) = by_waterfall(point);
        let fall = along > layout::CLIFF[0] - 2.5 && across < 3.5;
        let lip = Vec2::new(layout::RIVER[1].x, layout::RIVER[1].z);
        if fall || point.distance(lip) < BANKS[2] {
            continue;
        }
        let foot = drawn_ground(point).unwrap_or(0.0);
        place(
            &mut placed,
            point,
            foot - 0.03,
            shaped(seed, 0.35, 0.85),
            seed,
        );
    }
    placed
}

/// How many triangles each part of the backdrop draws: the foothills' ground, every plant but the tree ferns, which are
/// counted with the ferns ([`crate::ferns::triangles`]), the ranges, the water, above the waterfall too, the river's
/// stones, with the terrace's, the waterfall rock, the trail and the terrace.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Triangles {
    pub foothills: usize,
    pub plants: usize,
    pub ranges: usize,
    pub water: usize,
    pub stones: usize,
    pub rock: usize,
    pub trail: usize,
    pub terrace: usize,
}

impl Triangles {
    /// The backdrop's, counted from what it builds.
    pub fn counted() -> Self {
        let count = |model: &Model| {
            model
                .meshes
                .iter()
                .flat_map(|mesh| &mesh.primitives)
                .map(|primitive| primitive.indices.len() / 3)
                .sum::<usize>()
        };
        let plants = plants();
        Self {
            foothills: count(&model()),
            plants: Kind::ALL
                .iter()
                .filter_map(|kind| {
                    let model = kind.model()?;
                    Some(count(&model) * plants.iter().filter(|plant| plant.kind == *kind).count())
                })
                .sum(),
            ranges: ranges().len() / 3,
            water: (water().len() + upper_water().len()) / 3,
            stones: count(&stone()) * (stones().len() + crate::terrace::stones().len()),
            rock: count(&rock()),
            trail: count(&crate::trail::model()),
            terrace: count(&crate::terrace::model()),
        }
    }

    /// Them all together.
    pub fn total(&self) -> usize {
        self.foothills
            + self.plants
            + self.ranges
            + self.water
            + self.stones
            + self.rock
            + self.trail
            + self.terrace
    }
}

#[cfg(test)]
mod tests;
