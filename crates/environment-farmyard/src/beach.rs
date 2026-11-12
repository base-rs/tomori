// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The shingle beach beside the trail where it passes the river, by the `riverbank` viewpoint: a slope of grey shingle
//! from the trail's edge down into the river's channel to the water's edge, the one surface that reaches its water, with
//! river stones lying on it, slabs, cobbles, long and angular ones, from a hand's breadth across to as much as a hand
//! might lift, and beside them a handful of much bigger building stones, flagstones and blocks, for building little
//! structures, which participants take with either hand, carry, throw and stack into cairns, simulated by the
//! farmyard's server side with `tomori-physics` and drawn alike by every client. See `design/farmyard.md` and
//! `docs/proposals/physics-and-stacking.md`.
//!
//! What both halves share is here: where the beach is and how high its surface stands; each stone's hull, from which
//! it is both drawn and collided; what the stones rest on and the river's water; and the models of the beach and its
//! stones. The farmyard's server side runs the world of them, [`crate::side`]; its client crate mirrors it.
use std::f32::consts::TAU;

use glam::{Quat, Vec2, Vec3};
use tomori_assets::{Limits, Material, Mesh, Model, Node, Transform};
use tomori_physics::{Body, Placement, Settings, Static, Water};

use crate::backdrop::{self, facing};
use crate::layout;
use crate::props::scatter;
use crate::shapes::Shapes;
use crate::trail;

/// The point of the trail's middle the beach lies beside, by the `riverbank` viewpoint, which is the same point.
pub const AT: usize = 18;
/// How far out from the trail's middle, towards the river, the beach runs: from the trail's edge to a little into the
/// water, and how far either side along the trail.
pub const NEAR: f32 = trail::HALF;
pub const FAR: f32 = 5.1;
pub const HALF_WIDTH: f32 = 1.5;
/// How far apart the corners of its surface are, and how far above the ground as the backdrop draws it, so that the
/// ground never shows through the shingle.
pub const STEP: f32 = 0.25;
pub const RAISE: f32 = 0.015;
/// How many times its surface is smoothed, and how far above the ground as drawn it stays at least, once smoothed.
const SMOOTHING: usize = 6;
const FLOOR: f32 = 0.008;

/// Where the beach is: the trail's point beside it, the way along the trail, and the way out, towards the river.
pub fn frame() -> (Vec2, Vec2, Vec2) {
    trail::frame(AT)
}

/// How far along the trail the beach's middle is from [`AT`]'s point, so that it keeps clear of the exercise station's
/// clearing beside it.
const SHIFT: f32 = 0.4;

/// The point on the ground `out` towards the river from the trail's middle and `along` it from the beach's middle.
pub fn at(out: f32, along: f32) -> Vec2 {
    let (point, way, outward) = frame();
    point + outward * out + way * (along + SHIFT)
}

/// How far `point`, on the ground, is outside the beach's outline, or, inside it, how far within it, less than none.
pub fn from_beach(point: Vec2) -> f32 {
    let (middle, way, outward) = frame();
    let offset = point - middle;
    let across = Vec2::new(
        (offset.dot(way) - SHIFT).abs() - HALF_WIDTH,
        (offset.dot(outward) - (NEAR + FAR) * 0.5).abs() - (FAR - NEAR) * 0.5,
    );
    across.max(Vec2::ZERO).length() + across.max_element().min(0.0)
}

/// The beach's outline on the ground, its four corners.
pub fn outline() -> Vec<Vec2> {
    vec![
        at(NEAR, -HALF_WIDTH),
        at(FAR, -HALF_WIDTH),
        at(FAR, HALF_WIDTH),
        at(NEAR, HALF_WIDTH),
    ]
}

/// How many corners across and along its surface has, out from the trail and along it.
const COLUMNS: usize = ((FAR - NEAR) / STEP) as usize + 1;
const ROWS: usize = (2.0 * HALF_WIDTH / STEP) as usize + 1;

/// How high the beach's surface stands at its corner `point`: a little above the ground as it is drawn there, or as it
/// is shaped, whichever is higher, and above the trail where it meets it.
fn corner(point: Vec2) -> f32 {
    let drawn = backdrop::drawn_ground(point).unwrap_or(backdrop::MEADOW);
    let mut height = drawn.max(backdrop::ground(point)) + RAISE;
    if trail::from_middle(point) <= trail::HALF + 0.01 {
        height = height.max(trail::LIFT + 0.004);
    }
    height
}

/// The beach's surface: its corners, a grid out from the trail and along it, and its triangles, two to each cell,
/// as it is drawn and as the stones rest on it.
pub fn surface() -> (Vec<Vec3>, Vec<[u32; 3]>) {
    let mut corners = Vec::with_capacity(COLUMNS * ROWS);
    for row in 0..ROWS {
        for column in 0..COLUMNS {
            let out = (NEAR + column as f32 * STEP).min(FAR);
            let point = at(out, -HALF_WIDTH + row as f32 * STEP);
            corners.push(Vec3::new(point.x, corner(point), point.y));
        }
    }
    // Smoothed, so that where the slope bends it bends over a metre rather than at one edge, where a stone lying
    // across the bend would rock on it; and never below the ground as it is drawn, nor below the trail at its edge.
    let lowest: Vec<f32> = corners.iter().map(|corner| corner.y).collect();
    for _ in 0..SMOOTHING {
        let heights: Vec<f32> = corners.iter().map(|corner| corner.y).collect();
        for row in 0..ROWS {
            for column in 0..COLUMNS {
                let at = |row: usize, column: usize| heights[row * COLUMNS + column];
                let (up, down) = (row.saturating_sub(1), (row + 1).min(ROWS - 1));
                let (near, far) = (column.saturating_sub(1), (column + 1).min(COLUMNS - 1));
                let around =
                    (at(up, column) + at(down, column) + at(row, near) + at(row, far)) / 4.0;
                let index = row * COLUMNS + column;
                let floor = lowest[index] - RAISE + FLOOR;
                corners[index].y = ((at(row, column) + around) * 0.5).max(floor);
            }
        }
    }
    let mut triangles = Vec::new();
    for row in 0..ROWS - 1 {
        for column in 0..COLUMNS - 1 {
            let index = |row: usize, column: usize| (row * COLUMNS + column) as u32;
            let (a, b) = (index(row, column), index(row, column + 1));
            let (c, d) = (index(row + 1, column + 1), index(row + 1, column));
            if (row + column) % 2 == 0 {
                triangles.extend([[a, b, c], [a, c, d]]);
            } else {
                triangles.extend([[a, b, d], [b, c, d]]);
            }
        }
    }
    (corners, triangles)
}

/// The beach's surface, made once.
fn made() -> &'static (Vec<Vec3>, Vec<[u32; 3]>) {
    static SURFACE: std::sync::OnceLock<(Vec<Vec3>, Vec<[u32; 3]>)> = std::sync::OnceLock::new();
    SURFACE.get_or_init(surface)
}

/// How high the beach's surface stands at `point`, from its triangle there, or `None` beyond the beach.
pub fn height(point: Vec2) -> Option<f32> {
    let (corners, triangles) = made();
    triangles.iter().find_map(|triangle| {
        let [a, b, c] = triangle.map(|index| corners[index as usize]);
        let flat = |corner: Vec3| Vec2::new(corner.x, corner.z);
        let (a2, b2, c2) = (flat(a), flat(b), flat(c));
        let area = (b2 - a2).perp_dot(c2 - a2);
        let u = (b2 - point).perp_dot(c2 - point) / area;
        let v = (c2 - point).perp_dot(a2 - point) / area;
        let w = 1.0 - u - v;
        (u >= -1e-5 && v >= -1e-5 && w >= -1e-5).then_some(a.y * u + b.y * v + c.y * w)
    })
}

/// How far out from the trail's middle the beach is dry all along, a hand's breadth above the water, where its stones
/// lie to begin with.
pub fn dry() -> f32 {
    let level = water_level() + 0.1;
    (0..=100)
        .map(|step| NEAR + (FAR - NEAR) * step as f32 / 100.0)
        .take_while(|out| {
            [-HALF_WIDTH, 0.0, HALF_WIDTH]
                .iter()
                .all(|along| height(at(*out, *along)).is_some_and(|height| height > level))
        })
        .last()
        .unwrap_or(NEAR)
}

/// The river's water level beside the beach.
pub fn water_level() -> f32 {
    layout::nearest_river(at(FAR, 0.0)).0.y
}

/// The beach's shingle as ground participants walk on, which the farmyard's `walkable` derives its walkable ground from:
/// its triangles as they are drawn, from just beyond the trail's edge, which is the trail's to stand on, only where the
/// whole backdrop is within the far plane from the eyes of one standing there, which is its flat shingle beside the
/// trail, and short of where the river's water may spread, as everywhere in the farmyard, so that nobody lands in it.
/// Its stones down its slope are reached by pulling them along the ray, which reaches the water's edge from there.
pub fn ground() -> Vec<[Vec3; 3]> {
    let (corners, triangles) = made();
    let spread = layout::RIVER_HALF_WIDTH + backdrop::SPREAD + 0.1;
    let walked = |corner: &Vec3| {
        let point = Vec2::new(corner.x, corner.z);
        trail::from_middle(point) >= trail::HALF + 0.02
            && in_view(point)
            && layout::nearest_river(point).1 > spread
    };
    triangles
        .iter()
        .map(|triangle| triangle.map(|corner| corners[corner as usize]))
        .filter(|triangle| triangle.iter().all(walked))
        .collect()
}

/// How far away, at most, the backdrop may be from the eyes of one standing on the beach: two metres within the
/// headset's far plane of 50 m, and a little more to spare.
pub const VIEW: f32 = 47.5;

/// Whether the whole of the foothills' ground, as the backdrop draws it, is within [`VIEW`] of the eyes of one standing
/// at `point`.
fn in_view(point: Vec2) -> bool {
    static FOOTHILLS: std::sync::OnceLock<Vec<Vec3>> = std::sync::OnceLock::new();
    let foothills =
        FOOTHILLS.get_or_init(|| backdrop::model().meshes[0].primitives[0].positions.clone());
    let eyes = Vec3::new(point.x, tomori_environment::sightlines::EYES, point.y);
    foothills.iter().all(|corner| corner.distance(eyes) < VIEW)
}

/// The river's stones on and beside the beach, which participants keep clear of, as `(least, greatest)` corners.
pub fn solids() -> Vec<([f32; 3], [f32; 3])> {
    boulders()
        .iter()
        .map(|boulder| {
            let solid = boulder.solid();
            (solid.low, solid.high)
        })
        .collect()
}

/// How many river stones lie on the beach, numbered from 0; how many big building stones beside them, numbered after
/// them; and how many stones in all.
pub const RIVER: usize = 28;
pub const BUILDING: usize = 8;
pub const STONES: usize = RIVER + BUILDING;

/// The kinds of stone the beach has: as a river's shingle sorts them, flat slabs and discs, which cairns are built of,
/// rounded cobbles, long ones, and angular ones, broken rather than worn; and, to build with, flagstones, broad and
/// flat, for lintels, tables and roofs, and blocks, for pillars and walls.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Slab,
    Cobble,
    Long,
    Angular,
    Flagstone,
    Block,
}

/// Which kind stone `index` is: of every seven river stones, three slabs, two cobbles, a long one and an angular one;
/// and of the building stones, five flagstones and three blocks.
pub fn kind(index: usize) -> Kind {
    if building(index) {
        return match index - RIVER {
            2 | 5 | 7 => Kind::Block,
            _ => Kind::Flagstone,
        };
    }
    match index % 7 {
        0 | 2 | 5 => Kind::Slab,
        1 | 6 => Kind::Cobble,
        3 => Kind::Long,
        _ => Kind::Angular,
    }
}

/// Whether stone `index` is one of the big river stones, every other one, two to four hands' breadths across, as much
/// as one hand holds; the rest a hand's breadth to two.
pub fn big(index: usize) -> bool {
    !building(index) && index % 2 == 1
}

/// Whether stone `index` is one of the building stones, half a metre to three quarters across, after the river's.
pub fn building(index: usize) -> bool {
    index >= RIVER
}

/// How wide stone `index` is, its longest way across, 11.5 to 20 cm, or a big one, as its kind is, up to 37 cm for a
/// slab or a long one and less for the rounder, heavier kinds, so that none is more than a hand might lift, about 15
/// kg; and how high, as its kind is: a slab a quarter to a third of its width, a cobble half to two thirds, a long or
/// an angular one a third or more. A building stone is as [`hewn`] says.
pub fn size(index: usize) -> (f32, f32) {
    if building(index) {
        let (width, _, height) = hewn(index);
        return (width, height);
    }
    let seed = index as u32 * 7 + 3100;
    let kind = kind(index);
    let (least, more) = match (big(index), kind) {
        (false, _) => (0.115, 0.085),
        (true, Kind::Slab | Kind::Long) => (0.28, 0.09),
        (true, Kind::Angular) => (0.25, 0.08),
        (true, _) => (0.22, 0.06),
    };
    let width = least + scatter(seed) * more;
    let (least, more) = match kind {
        Kind::Slab => (0.24, 0.09),
        Kind::Cobble => (0.5, 0.15),
        Kind::Long => (0.34, 0.1),
        _ => (0.36, 0.12),
    };
    (width, width * (least + scatter(seed + 1) * more))
}

/// How wide building stone `index` is, its longest way across, how deep, the other way, and how high: a flagstone 55 to
/// 75 cm across, three fifths to three quarters as deep and 9 to 14 cm thick, and a block 34 to 44 cm across, three
/// quarters as deep or more and 32 to 35 cm high, so that two of them, or three in a row, hold up a flagstone as a
/// dolmen's pillars do, nearly level.
/// Their widest is within [`tomori_physics::HOLD`] of a hand that holds them by an edge.
pub fn hewn(index: usize) -> (f32, f32, f32) {
    let seed = index as u32 * 11 + 3500;
    let ((width, wider), (deep, deeper), (high, higher)) = match kind(index) {
        Kind::Block => ((0.34, 0.1), (0.74, 0.14), (0.32, 0.03)),
        _ => ((0.55, 0.2), (0.6, 0.15), (0.09, 0.05)),
    };
    let width = width + scatter(seed) * wider;
    (
        width,
        width * (deep + scatter(seed + 1) * deeper),
        high + scatter(seed + 2) * higher,
    )
}

/// How far in a building stone's edges are bevelled, at its bottom, about its middle and at its top, as fractions of
/// its half width and depth.
const BEVEL: f32 = 0.12;

/// The points of building stone `index`'s hull, about its middle, the same every time: a box of its [`hewn`] size, its
/// edges bevelled and its corners nudged, as a stone split along its bedding and dressed roughly is; its bottom a face
/// of four, flat, its top a face of four, flat and very nearly level, so that a flagstone laid across two blocks lies
/// level on them, and between them a ring of eight about its middle, nudged in and up or down. Sixteen points.
fn dressed(index: usize) -> Vec<Vec3> {
    let (width, depth, height) = hewn(index);
    let (x, z) = (width * 0.5, depth * 0.5);
    let seed = index as u32 * 97 + 3600;
    let nudge = |offset: u32| scatter(seed + offset);
    let (tilt_x, tilt_z) = (nudge(0) - 0.5, nudge(1) - 0.5);
    let corners = [(1.0, 1.0), (-1.0, 1.0), (-1.0, -1.0), (1.0, -1.0)];
    let mut points = Vec::with_capacity(16);
    for (level, base) in [(-0.5, 10), (0.5, 20)] {
        for (corner, (across, along)) in corners.iter().enumerate() {
            let inset = 1.0 - BEVEL * (0.7 + nudge(base + corner as u32) * 0.6);
            let (px, pz) = (across * x * inset, along * z * inset);
            // Its top tilted no more than about a degree, its bottom flat.
            let tilt = if level > 0.0 {
                (px * tilt_x + pz * tilt_z) * 0.03
            } else {
                0.0
            };
            points.push(Vec3::new(px, level * height + tilt, pz));
        }
    }
    for (corner, (across, along)) in corners.iter().enumerate() {
        for (side, (wide, deep)) in [(1.0, 1.0 - 2.0 * BEVEL), (1.0 - 2.0 * BEVEL, 1.0)]
            .iter()
            .enumerate()
        {
            let at = 30 + corner as u32 * 4 + side as u32 * 2;
            let reach = 1.0 - nudge(at) * 0.05;
            let level = (nudge(at + 1) - 0.5) * 0.24 * height;
            points.push(Vec3::new(
                across * x * wide * reach,
                level,
                along * z * deep * reach,
            ));
        }
    }
    points
}

/// One ring of a stone's points, from its bottom up: how many corners, at what height, as a fraction of the stone's
/// from its middle, how far out, as a fraction of its half width, and turned how far, as a fraction of a corner.
struct Ring {
    corners: usize,
    level: f32,
    reach: f32,
    turned: f32,
}

/// The rings of a stone of `kind`: a face of four at the bottom, flat, and one at the top, flat but tilted, since Rapier
/// rests a face of more than four corners on another unsteadily, so that a stone of them shakes off the one put on it;
/// and between them a broad ring, or, a cobble's, three, rounding it.
fn rings(kind: Kind) -> Vec<Ring> {
    let ring = |corners, level, reach, turned| Ring {
        corners,
        level,
        reach,
        turned,
    };
    match kind {
        Kind::Slab => vec![
            ring(4, -0.5, 0.7, 0.5),
            ring(6, 0.0, 1.0, 0.0),
            ring(4, 0.5, 0.66, 0.0),
        ],
        Kind::Long => vec![
            ring(4, -0.5, 0.64, 0.5),
            ring(6, 0.0, 1.0, 0.0),
            ring(4, 0.5, 0.6, 0.0),
        ],
        Kind::Cobble => vec![
            ring(4, -0.5, 0.46, 0.5),
            ring(6, -0.3, 0.87, 0.5),
            ring(6, 0.0, 1.0, 0.0),
            ring(6, 0.3, 0.87, 0.5),
            ring(4, 0.5, 0.44, 0.0),
        ],
        _ => vec![
            ring(4, -0.5, 0.72, 0.3),
            ring(5, 0.0, 1.0, 0.0),
            ring(4, 0.5, 0.6, 0.6),
        ],
    }
}

/// How much each corner of a stone of `kind` is nudged in and along its ring, and up or down from it, as fractions;
/// how much it is squashed across, at least and how much more; and how far its top is tilted.
fn roughness(kind: Kind) -> (f32, f32, f32, (f32, f32), f32) {
    match kind {
        Kind::Slab => (0.08, 0.0, 0.0, (0.72, 0.22), 0.16),
        Kind::Long => (0.08, 0.0, 0.0, (0.46, 0.14), 0.16),
        Kind::Cobble => (0.05, 0.0, 0.0, (0.84, 0.13), 0.1),
        _ => (0.18, 0.12, 0.24, (0.66, 0.2), 0.3),
    }
}

/// The points of stone `index`'s hull, about its middle, the same every time, ring by ring from its bottom up, as its
/// kind's rings are, each corner nudged its own way, the whole squashed across, a little longer one way than the other
/// or, a long one, much longer: its bottom flat and its top flat but a little tilted, each its own way, as stones
/// chosen for a cairn are, so that it lies still where it is put and a stone put on it lies still on it. It is drawn
/// from the same points, and collided as their convex hull, at most [`tomori_physics::HULL_POINTS`]. A building stone
/// is [`dressed`] rather than worn.
pub fn stone(index: usize) -> Vec<Vec3> {
    if building(index) {
        return dressed(index);
    }
    let (width, height) = size(index);
    let kind = kind(index);
    let radius = width * 0.5;
    let seed = index as u32 * 97 + 3200;
    let (inward, along_ring, upward, (squashed, squash_more), tilting) = roughness(kind);
    let squash = squashed + scatter(seed) * squash_more;
    let (x, z) = (scatter(seed + 70) - 0.5, scatter(seed + 71) - 0.5);
    let rings = rings(kind);
    let last = rings.len() - 1;
    let mut points = Vec::new();
    for (number, ring) in rings.iter().enumerate() {
        for corner in 0..ring.corners {
            let nudge = |offset: u32| scatter(seed + (number * 8 + corner) as u32 * 3 + offset + 1);
            let face = number == 0 || number == last;
            let turn = (corner as f32 + ring.turned + (nudge(1) - 0.5) * along_ring)
                / ring.corners as f32
                * TAU;
            let reach = radius * ring.reach * (1.0 - inward + nudge(0) * inward);
            let (across, along) = (turn.cos() * reach, turn.sin() * reach * squash);
            let level = if face {
                ring.level
            } else {
                ring.level + (nudge(2) - 0.5) * upward
            };
            let tilt = if number == last {
                (across * x + along * z) * tilting
            } else {
                0.0
            };
            points.push(Vec3::new(across, level * height + tilt, along));
        }
    }
    points
}

/// The faces of the convex hull of `points`, each its outward normal and its corners, in turn about it: every plane
/// through three of them that has none of the rest beyond it, with every point on it.
fn faces(points: &[Vec3]) -> Vec<(Vec3, Vec<usize>)> {
    let scale = points
        .iter()
        .map(|point| point.length())
        .fold(0.0, f32::max);
    let close = scale * 1e-5;
    let mut faces: Vec<(Vec3, Vec<usize>)> = Vec::new();
    for a in 0..points.len() {
        for b in a + 1..points.len() {
            for c in b + 1..points.len() {
                // Three in a line make no plane, every point on it, and so no face.
                let normal = (points[b] - points[a])
                    .cross(points[c] - points[a])
                    .normalize_or_zero();
                let beyond = |point: &Vec3| normal.dot(*point - points[a]);
                let above = points.iter().any(|point| beyond(point) > close);
                let below = points.iter().any(|point| beyond(point) < -close);
                if above == below {
                    continue;
                }
                let outward = if above { -normal } else { normal };
                let on: Vec<usize> = (0..points.len())
                    .filter(|point| beyond(&points[*point]).abs() <= close)
                    .collect();
                if faces.iter().any(|(_, corners)| {
                    corners.len() == on.len() && on.iter().all(|point| corners.contains(point))
                }) {
                    continue;
                }
                // In turn about its middle, anticlockwise seen from outside.
                let middle = on.iter().map(|point| points[*point]).sum::<Vec3>() / on.len() as f32;
                let first = (points[on[0]] - middle).normalize();
                let second = outward.cross(first);
                let mut corners = on;
                corners.sort_by(|p, q| {
                    let angle = |point: usize| {
                        let offset = points[point] - middle;
                        offset.dot(second).atan2(offset.dot(first))
                    };
                    angle(*p).total_cmp(&angle(*q))
                });
                faces.push((outward, corners));
            }
        }
    }
    faces
}

/// The triangles each face of `points`' hull is drawn in, a fan about its first corner, each turned outward.
fn hull(points: &[Vec3]) -> Vec<[usize; 3]> {
    faces(points)
        .into_iter()
        .flat_map(|(_, corners)| {
            (1..corners.len() - 1)
                .map(|corner| [corners[0], corners[corner], corners[corner + 1]])
                .collect::<Vec<_>>()
        })
        .collect()
}

/// The triangles stone `index` is drawn in, by the points of its hull, each turned outward, its convex hull's faces
/// exactly, so that it is drawn as it collides; none for a stone the beach does not have.
pub fn stone_triangles(index: usize) -> &'static [[usize; 3]] {
    static HULLS: std::sync::OnceLock<Vec<Vec<[usize; 3]>>> = std::sync::OnceLock::new();
    HULLS
        .get_or_init(|| (0..STONES).map(|index| hull(&stone(index))).collect())
        .get(index)
        .map_or(&[], Vec::as_slice)
}

/// How much stone `index` holds, in cubic metres, from its triangles about its middle.
pub fn volume(index: usize) -> f32 {
    let points = stone(index);
    stone_triangles(index)
        .iter()
        .map(|[a, b, c]| points[*a].dot(points[*b].cross(points[*c])) / 6.0)
        .sum()
}

/// How far a point `local`, about stone `index`'s middle, is outside its hull: as far as it is beyond the plane of the
/// face it is farthest beyond, or none within it, which is the distance to a face and a little less near an edge or a
/// corner.
pub fn outside(index: usize, local: Vec3) -> f32 {
    let points = stone(index);
    stone_triangles(index)
        .iter()
        .map(|[a, b, c]| {
            let normal = (points[*b] - points[*a])
                .cross(points[*c] - points[*a])
                .normalize_or_zero();
            normal.dot(local - points[*a])
        })
        .fold(0.0, f32::max)
}

/// The river's stones beside the beach, which its stones rest against and on, as the backdrop places them.
fn boulders() -> Vec<backdrop::Stone> {
    let middle = at((NEAR + FAR) * 0.5, 0.0);
    backdrop::stones()
        .into_iter()
        .filter(|stone| Vec2::new(stone.at.x, stone.at.z).distance(middle) < REGION)
        .collect()
}

/// How far out from the trail's middle the beach's gentle slope ends, and its steep bank begins, roughly.
const GENTLE: f32 = NEAR + 2.6;

/// How far from the beach's middle what the stones may rest on reaches: the ground, the trail, the river's bed and its
/// far bank; beyond, a stone thrown is lost, and returns to where it began.
const REGION: f32 = 7.0;

/// How far out from the trail's middle the building stones lie: on the flat shingle beyond where participants stand,
/// and where it begins to slope, clear of the trail and of the shingle that is walked; and where along the beach the
/// pile of flagstones lies, towards one end, and the blocks, in a row towards the other.
pub const BUILDING_OUT: f32 = NEAR + 1.32;
pub const PILE_ALONG: f32 = -0.9;
pub const BLOCKS_ALONG: [f32; 3] = [0.45, 0.85, 1.25];

/// The highest the shingle stands beneath stone `index` placed at `at`, where it is above the shingle.
fn beneath(index: usize, at: &Placement) -> f32 {
    stone(index)
        .into_iter()
        .filter_map(|point| {
            let point = at.transform_point(point);
            height(Vec2::new(point.x, point.z))
        })
        .fold(f32::NEG_INFINITY, f32::max)
}

/// How far below and above its middle stone `index`'s lowest and highest points are, turned only about the upright.
fn extent(index: usize) -> (f32, f32) {
    stone(index)
        .into_iter()
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(low, high), point| {
            (low.min(point.y), high.max(point.y))
        })
}

/// Where the building stones lie to begin with, roughly, as the server side lets them settle, by their numbers: the
/// flagstones in a pile, the broadest at the bottom, each a little turned and off the one below it, as they would be
/// stacked by hand, and the blocks standing in a row, each its long way out from the trail, so that the beach is kept
/// clear for the river's stones and nobody walks into them.
fn building_homes() -> Vec<(usize, Placement)> {
    let (_, way, _) = frame();
    let along_beach = Quat::from_rotation_y((-way.y).atan2(way.x));
    let mut flagstones: Vec<usize> = (RIVER..STONES)
        .filter(|index| kind(*index) == Kind::Flagstone)
        .collect();
    flagstones.sort_by(|a, b| size(*b).0.total_cmp(&size(*a).0));
    let mut homes = Vec::with_capacity(BUILDING);
    let mut top = f32::NEG_INFINITY;
    for (level, index) in flagstones.into_iter().enumerate() {
        let seed = index as u32 * 13 + 3700;
        let point = at(
            BUILDING_OUT + (scatter(seed) - 0.5) * 0.03,
            PILE_ALONG + (scatter(seed + 1) - 0.5) * 0.03,
        );
        let turn = along_beach * Quat::from_rotation_y((scatter(seed + 2) - 0.5) * 0.12);
        let (low, high) = extent(index);
        let mut home = Placement::new(Vec3::new(point.x, 0.0, point.y), turn);
        let below = if level == 0 {
            beneath(index, &home) + 0.005
        } else {
            top + 0.003
        };
        home.position.y = below - low;
        top = home.position.y + high;
        homes.push((index, home));
    }
    let blocks = (RIVER..STONES).filter(|index| kind(*index) == Kind::Block);
    for (index, along) in blocks.zip(BLOCKS_ALONG) {
        let point = at(BUILDING_OUT, along);
        let turn = along_beach
            * Quat::from_rotation_y(
                std::f32::consts::FRAC_PI_2 + (scatter(index as u32 + 3800) - 0.5) * 0.2,
            );
        let mut home = Placement::new(Vec3::new(point.x, 0.0, point.y), turn);
        home.position.y = beneath(index, &home) + 0.005 - extent(index).0;
        homes.push((index, home));
    }
    homes.sort_by_key(|(index, _)| *index);
    homes
}

/// Where stone `index` lies to begin with, roughly, as the server side lets it settle: the river's scattered over the
/// beach, more of them near the water, apart from each other, from the building stones and from the river's own
/// stones, each turned as it fell; and the building stones as [`building_homes`] lays them.
pub fn homes() -> Vec<Placement> {
    let boulders = boulders();
    let dry = dry();
    let building = building_homes();
    let mut placed: Vec<(Vec2, f32)> = building
        .iter()
        .map(|(index, home)| {
            (
                Vec2::new(home.position.x, home.position.z),
                size(*index).0 * 0.5,
            )
        })
        .collect();
    let mut homes = Vec::with_capacity(STONES);
    for index in 0..RIVER {
        let (width, tall) = size(index);
        let radius = width * 0.5;
        let mut attempt = 0u32;
        let point = loop {
            let seed = index as u32 * 131 + attempt * 7 + 5000;
            // Clear of the trail's raised edge and the beach's own, however big; and a big one on the flat shingle or
            // the gentle slope below it, not the steep bank, where its broad bottom would bridge the bend.
            let farthest = if big(index) { GENTLE } else { dry - 0.2 };
            let (nearest, farthest) = (NEAR + 0.15 + radius, farthest - radius);
            let out = nearest + (farthest - nearest) * scatter(seed).sqrt();
            let along = (scatter(seed + 1) - 0.5) * 2.0 * (HALF_WIDTH - 0.1 - radius);
            let point = at(out, along);
            attempt += 1;
            let clear = placed
                .iter()
                .all(|(other, reach)| other.distance(point) > reach + radius + 0.04)
                && boulders.iter().all(|boulder| {
                    Vec2::new(boulder.at.x, boulder.at.z).distance(point)
                        > boulder.size.x * 0.55 + radius
                });
            if clear || attempt > 200 {
                break point;
            }
        };
        placed.push((point, radius));
        let ground = height(point).expect("on the beach");
        let turn = Quat::from_rotation_y(scatter(index as u32 + 5900) * TAU);
        homes.push(Placement::new(
            Vec3::new(point.x, ground + tall * 0.5 + 0.01, point.y),
            turn,
        ));
    }
    homes.extend(building.into_iter().map(|(_, home)| home));
    homes
}

/// The beach's stones as bodies, numbered from 0, each of granite, where it lies to begin with, let go no faster than
/// its [`heft`] lets a hand throw it.
pub fn bodies() -> Vec<Body> {
    homes()
        .into_iter()
        .enumerate()
        .map(|(index, home)| {
            Body::convex(index as u32, stone(index))
                .at(home)
                .thrown_at_most(fastest(index))
        })
        .collect()
}

/// Granite's density, in kilograms a cubic metre, which the beach's stones are of.
pub const GRANITE: f32 = 2_650.0;
/// The mass a hand throws as fast as it moves, in kilograms, and the least of the hand's speed a stone leaves it with.
const HEFTED: f32 = 1.5;
const HEAVIEST: f32 = 0.25;

/// How much of the hand's speed stone `index` leaves it with: all of it, for a stone of [`HEFTED`] or less, and less
/// the heavier it is, down to [`HEAVIEST`], so that a big stone thrown as a small one is goes slower and less far, as
/// it would, and a building stone, of 50 kg or more, barely leaves the hand.
pub fn heft(index: usize) -> f32 {
    let mass = volume(index) * GRANITE;
    (HEFTED / mass).powf(0.3).clamp(HEAVIEST, 1.0)
}

/// The fastest stone `index` leaves a hand, in metres a second: [`tomori_physics::RELEASE_SPEED`] as its [`heft`]
/// lessens it, which the server side holds a letting go to, as it holds every one to that.
pub fn fastest(index: usize) -> f32 {
    tomori_physics::RELEASE_SPEED * heft(index)
}

/// What the stones rest on: the beach's surface as it is drawn; the ground about it, the trail, the river's bed and its
/// far bank, as the backdrop draws them; and the river's stones beside it, each its own hull.
pub fn colliders() -> Vec<Static> {
    let (corners, triangles) = surface();
    let mut colliders = vec![Static::mesh(corners, triangles)];
    let middle = at((NEAR + FAR) * 0.5, 0.0);
    let spacing = 0.5;
    let columns = (REGION * 2.0 / spacing) as usize + 1;
    let corner = middle - Vec2::splat(REGION);
    let heights: Vec<f32> = (0..columns * columns)
        .map(|index| {
            let point =
                corner + Vec2::new((index % columns) as f32, (index / columns) as f32) * spacing;
            let ground = backdrop::drawn_ground(point).unwrap_or(backdrop::MEADOW);
            if trail::from_middle(point) < trail::HALF {
                ground.max(trail::LIFT)
            } else {
                ground
            }
        })
        .collect();
    colliders.push(Static::heights(corner, spacing, columns, &heights));
    let corners = backdrop::stone_corners();
    colliders.extend(boulders().into_iter().map(|boulder| {
        let transform = boulder.transform();
        Static::convex(
            corners
                .iter()
                .map(|point| transform.transform_point3(*point))
                .collect(),
        )
    }));
    colliders
}

/// The river's water beside the beach, in bands along it at its level in each, which slows and buoys a stone thrown in
/// and, once it has rested there a minute, takes it back to where it began on the beach.
pub fn waters() -> Vec<Water> {
    let middle = at((NEAR + FAR) * 0.5, 0.0);
    let bands = 5;
    let band = REGION * 2.0 / bands as f32;
    (0..bands)
        .map(|index| {
            let low = middle.y - REGION + index as f32 * band;
            let level = layout::nearest_river(Vec2::new(middle.x + 2.0, low + band * 0.5))
                .0
                .y;
            let outline = vec![
                Vec2::new(middle.x - REGION, low),
                Vec2::new(middle.x + REGION, low),
                Vec2::new(middle.x + REGION, low + band),
                Vec2::new(middle.x - REGION, low + band),
            ];
            Water::below(level, outline).returning(RETURNS)
        })
        .collect()
}

/// What changed of the beach's stones, as the farmyard's server side sends it to everyone in the space: the farmyard's
/// clock, and what changed. A client from before the beach reads it as neither a chicken's nor a stick's, and ignores it.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Event {
    pub clock: u64,
    pub stones: tomori_physics::message::Update,
}

/// The beach's stones as the farmyard shares them, in its state beside the flock's, or none in a state from before them.
pub fn shared(state: &serde_json::Value) -> tomori_physics::message::State {
    serde_json::from_value(state["stones"].clone()).unwrap_or_default()
}

/// How long a stone rests in the river before it comes back to the beach, in milliseconds.
pub const RETURNS: u64 = 60_000;

/// How the beach's world simulates: as `tomori-physics` does by default, a stone lost beyond its region going back, but
/// with every contact against the shingle's triangles solved against its own, [`Settings::unclustered`], so that a
/// building stone lying across a bend of a degree or two in the shingle rests rather than creeping along it.
pub fn settings() -> Settings {
    let middle = at((NEAR + FAR) * 0.5, 0.0);
    Settings {
        home: Vec3::new(middle.x, 0.0, middle.y),
        lost_beyond: REGION,
        ..Settings::default()
    }
    .unclustered()
}

/// The colours of the beach's shingle and its stones.
pub mod colors {
    use glam::Vec3;

    pub const SHINGLE: Vec3 = Vec3::new(0.2, 0.19, 0.17);
    pub const STONE: Vec3 = Vec3::new(0.3, 0.29, 0.27);
}

/// How much brighter or darker each of the beach's stones is drawn, and how much warmer or cooler.
fn shade(index: usize) -> Vec3 {
    let light = 0.7 + scatter(index as u32 * 5 + 6000) * 0.55;
    let warm = (scatter(index as u32 * 5 + 6001) - 0.5) * 0.08;
    colors::STONE * light + Vec3::new(warm, warm * 0.4, -warm)
}

/// The beach's shingle, as a model of one node, each of its triangles a little lighter or darker, as shingle is.
pub fn model() -> Model {
    let (corners, triangles) = surface();
    let mut shapes = Shapes::new();
    for (index, triangle) in triangles.iter().enumerate() {
        let points = triangle.map(|corner| corners[corner as usize]);
        let light = 0.85 + scatter(index as u32 + 6200) * 0.3;
        facing(&mut shapes, &points, Vec3::Y, colors::SHINGLE * light);
    }
    backdrop::built("beach", shapes)
}

/// The name of the node of the stones' model that draws stone `index`.
pub fn stone_node(index: usize) -> String {
    format!("stone-{index}")
}

/// The beach's stones, as a model of a node for each, about its middle, of which an instance shows one.
pub fn stones_model() -> Model {
    let mut nodes = Vec::with_capacity(STONES);
    let mut meshes = Vec::with_capacity(STONES);
    for index in 0..STONES {
        let points = stone(index);
        let mut shapes = Shapes::new();
        for triangle in stone_triangles(index) {
            let corners = triangle.map(|point| points[point]);
            let outward = (corners[0] + corners[1] + corners[2]) / 3.0;
            let light =
                0.92 + scatter((index * 64 + triangle[0] * 3 + triangle[2]) as u32 + 6400) * 0.16;
            facing(&mut shapes, &corners, outward, shade(index) * light);
        }
        nodes.push(Node::new(stone_node(index), Transform::IDENTITY).with_mesh(index));
        meshes.push(Mesh::new(stone_node(index), vec![shapes.primitive(0)]));
    }
    let roots = (0..nodes.len()).collect();
    Model::built(
        nodes,
        roots,
        meshes,
        vec![Material::new("plain", glam::Vec4::ONE)],
        &Limits::default(),
    )
    .expect("the stones are built within the limits")
}

/// How far along the ray from `origin` going `direction`, a unit vector, it meets stone `index` placed at `placement`, by
/// its triangles, if it does.
pub fn met(origin: Vec3, direction: Vec3, index: usize, placement: &Placement) -> Option<f32> {
    let (width, height) = size(index);
    // Nowhere near its sphere, nowhere near it.
    let offset = placement.position - origin;
    let along = offset.dot(direction);
    let bound = width.max(height) * 0.6;
    if (offset - direction * along).length() > bound || along < -bound {
        return None;
    }
    let points: Vec<Vec3> = stone(index)
        .iter()
        .map(|point| placement.transform_point(*point))
        .collect();
    stone_triangles(index)
        .iter()
        .filter_map(|[a, b, c]| crossing(origin, direction, [points[*a], points[*b], points[*c]]))
        .min_by(f32::total_cmp)
}

/// Where along a ray from `origin` going `direction` it crosses the triangle `corners`, either side, if it does.
fn crossing(origin: Vec3, direction: Vec3, [a, b, c]: [Vec3; 3]) -> Option<f32> {
    let (ab, ac) = (b - a, c - a);
    let p = direction.cross(ac);
    let determinant = ab.dot(p);
    if determinant.abs() < 1e-9 {
        return None;
    }
    let inverse = 1.0 / determinant;
    let t = origin - a;
    let u = t.dot(p) * inverse;
    let q = t.cross(ab);
    let v = direction.dot(q) * inverse;
    let along = ac.dot(q) * inverse;
    (u >= 0.0 && v >= 0.0 && u + v <= 1.0 && along >= 0.0).then_some(along)
}

/// The most triangles the beach and its stones draw together, which the tests check.
pub const BUDGET: usize = 1_600;

#[cfg(test)]
mod tests;
