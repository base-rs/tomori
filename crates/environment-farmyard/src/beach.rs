// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The shingle beach beside the trail where it passes the river, by the `riverbank` viewpoint: a slope of grey shingle
//! from the trail's edge down into the river's channel to the water's edge, the one surface that reaches its water, with
//! river stones lying on it, flattish and of every size from a hand's breadth to two, which participants take with
//! either hand, carry, throw and stack into cairns, simulated by the farmyard's server side with `tomori-physics` and
//! drawn alike by every client. See `design/farmyard.md` and `docs/proposals/physics-and-stacking.md`.
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

/// The point on the ground `out` towards the river from the trail's middle and `along` it.
pub fn at(out: f32, along: f32) -> Vec2 {
    let (point, way, outward) = frame();
    point + outward * out + way * along
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

/// Where participants may stand on the beach, as flat cells over its slope, each at the height of its middle, so that
/// none stands more than a few centimetres from the shingle: down to just above the water's edge. See `locomotion.md`;
/// walkable ground derived from what is drawn would replace them.
pub fn walkable() -> Vec<(Vec<Vec2>, f32)> {
    const OUT: usize = 12;
    const ALONG: usize = 5;
    let edge = (0..=40)
        .map(|step| NEAR + (FAR - NEAR) * step as f32 / 40.0)
        .find(|out| height(at(*out, HALF_WIDTH)).is_none_or(|height| height < water_level() + 0.05))
        .unwrap_or(FAR);
    let width = HALF_WIDTH - 0.05;
    let mut cells = Vec::with_capacity(OUT * ALONG);
    for band in 0..OUT {
        let (from, to) = (
            NEAR + (edge - NEAR) * band as f32 / OUT as f32,
            NEAR + (edge - NEAR) * (band + 1) as f32 / OUT as f32,
        );
        for strip in 0..ALONG {
            let (left, right) = (
                -width + 2.0 * width * strip as f32 / ALONG as f32,
                -width + 2.0 * width * (strip + 1) as f32 / ALONG as f32,
            );
            let middle = height(at((from + to) * 0.5, (left + right) * 0.5)).expect("on the beach");
            cells.push((
                vec![at(from, left), at(to, left), at(to, right), at(from, right)],
                middle,
            ));
        }
    }
    cells
}

/// How many stones lie on the beach.
pub const STONES: usize = 28;
/// How many corners a stone's middle ring has, and its bottom's and top's: four, since Rapier rests a face of more than
/// four corners on another unsteadily, so that a stone of them shakes off the one put on it. And so how many points its
/// hull has.
const SIDES: usize = 6;
const FACE: usize = 4;
pub const STONE_POINTS: usize = SIDES + FACE * 2;

/// How wide stone `index` is, a hand's breadth to two, and how high, a little over a third of that to a half, so that
/// their masses are within a ratio of 1 to 10 of each other, as an iterative solver stacks well.
pub fn size(index: usize) -> (f32, f32) {
    let width = 0.115 + scatter(index as u32 * 7 + 3100) * 0.105;
    let height = width * (0.38 + scatter(index as u32 * 7 + 3101) * 0.12);
    (width, height)
}

/// Where each corner of a stone's rings is about its middle, as a fraction of a turn: its bottom's, its middle's and its
/// top's, the faces turned half a corner from each other and from the middle.
fn turns() -> [Vec<f32>; 3] {
    let ring = |count: usize, offset: f32| {
        (0..count)
            .map(|corner| (corner as f32 + offset) / count as f32)
            .collect()
    };
    [ring(FACE, 0.5), ring(SIDES, 0.0), ring(FACE, 0.0)]
}

/// The points of stone `index`'s hull, about its middle, the same every time: a broad ring of six about its middle, a
/// little longer one way than the other, and a smaller face of four below and above it, its bottom flat and its top flat
/// but a little tilted, each its own way, as stones chosen for a cairn are, so that it lies still where it is put and a
/// stone put on it lies still on it. It is drawn from the same points, and collided as their convex hull.
pub fn stone(index: usize) -> Vec<Vec3> {
    let (width, height) = size(index);
    let radius = width * 0.5;
    let seed = index as u32 * 97 + 3200;
    let squash = 0.72 + scatter(seed) * 0.2;
    let (x, z) = (scatter(seed + 70) - 0.5, scatter(seed + 71) - 0.5);
    let mut points = Vec::with_capacity(STONE_POINTS);
    for (ring, (turns, (level, reach))) in turns()
        .iter()
        .zip([(-0.5f32, 0.66f32), (0.0, 1.0), (0.5, 0.64)])
        .enumerate()
    {
        for (corner, turn) in turns.iter().enumerate() {
            let turn = turn * TAU;
            let nudge = scatter(seed + (ring * SIDES + corner) as u32 + 1);
            let reach = radius * reach * (0.92 + nudge * 0.08);
            let (across, along) = (turn.cos() * reach, turn.sin() * reach * squash);
            let tilt = if level > 0.0 {
                (across * x + along * z) * 0.16
            } else {
                0.0
            };
            points.push(Vec3::new(across, level * height + tilt, along));
        }
    }
    points
}

/// The triangles each stone is drawn in, by the points of its hull, each turned outward: two across its bottom and two
/// across its top, and a band from each face to its middle ring, zipped about them in turn.
pub fn stone_triangles() -> Vec<[usize; 3]> {
    let [bottom, middle, top] = turns();
    let (low, mid, high) = (0, FACE, FACE + SIDES);
    let mut triangles = vec![
        [low, low + 1, low + 2],
        [low, low + 2, low + 3],
        [high, high + 1, high + 2],
        [high, high + 2, high + 3],
    ];
    // A face's ring and the middle's, zipped: each step takes the next corner of whichever ring comes first round.
    for (face, start) in [(&bottom, low), (&top, high)] {
        let (mut i, mut j) = (0, 0);
        while i < face.len() || j < middle.len() {
            let next_face = face[0] + (i + 1) as f32 / face.len() as f32;
            let next_middle = (j + 1) as f32 / middle.len() as f32;
            let corner = |ring: usize, count: usize, index: usize| ring + index % count;
            if j == middle.len() || (i < face.len() && next_face <= next_middle) {
                triangles.push([
                    corner(start, FACE, i),
                    corner(start, FACE, i + 1),
                    corner(mid, SIDES, j),
                ]);
                i += 1;
            } else {
                triangles.push([
                    corner(start, FACE, i),
                    corner(mid, SIDES, j + 1),
                    corner(mid, SIDES, j),
                ]);
                j += 1;
            }
        }
    }
    // Each turned outward, as a stone's points show which way.
    let points = stone(0);
    triangles
        .into_iter()
        .map(|[a, b, c]| {
            let normal = (points[b] - points[a]).cross(points[c] - points[a]);
            if normal.dot(points[a] + points[b] + points[c]) < 0.0 {
                [a, c, b]
            } else {
                [a, b, c]
            }
        })
        .collect()
}

/// The river's stones beside the beach, which its stones rest against and on, as the backdrop places them.
fn boulders() -> Vec<backdrop::Stone> {
    let middle = at((NEAR + FAR) * 0.5, 0.0);
    backdrop::stones()
        .into_iter()
        .filter(|stone| Vec2::new(stone.at.x, stone.at.z).distance(middle) < REGION)
        .collect()
}

/// How far from the beach's middle what the stones may rest on reaches: the ground, the trail, the river's bed and its
/// far bank; beyond, a stone thrown is lost, and returns to where it began.
const REGION: f32 = 7.0;

/// Where stone `index` lies to begin with, roughly, as the server side lets it settle: scattered over the beach, more
/// of them near the water, apart from each other and from the river's stones, each turned as it fell.
pub fn homes() -> Vec<Placement> {
    let boulders = boulders();
    let dry = dry();
    let mut placed: Vec<(Vec2, f32)> = Vec::new();
    let mut homes = Vec::with_capacity(STONES);
    for index in 0..STONES {
        let (width, tall) = size(index);
        let radius = width * 0.5;
        let mut attempt = 0u32;
        let point = loop {
            let seed = index as u32 * 131 + attempt * 7 + 5000;
            let out = NEAR + 0.3 + (dry - NEAR - 0.5) * scatter(seed).sqrt();
            let along = (scatter(seed + 1) - 0.5) * 2.0 * (HALF_WIDTH - 0.2);
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
    homes
}

/// The beach's stones as bodies, numbered from 0, each of granite, where it lies to begin with.
pub fn bodies() -> Vec<Body> {
    homes()
        .into_iter()
        .enumerate()
        .map(|(index, home)| Body::convex(index as u32, stone(index)).at(home))
        .collect()
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

/// How long a stone rests in the river before it comes back to the beach, in milliseconds.
pub const RETURNS: u64 = 60_000;

/// How the beach's world simulates: as `tomori-physics` does by default, a stone lost beyond its region going back.
pub fn settings() -> Settings {
    let middle = at((NEAR + FAR) * 0.5, 0.0);
    Settings {
        home: Vec3::new(middle.x, 0.0, middle.y),
        lost_beyond: REGION,
        ..Settings::default()
    }
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
    let triangles = stone_triangles();
    let mut nodes = Vec::with_capacity(STONES);
    let mut meshes = Vec::with_capacity(STONES);
    for index in 0..STONES {
        let points = stone(index);
        let mut shapes = Shapes::new();
        for triangle in &triangles {
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
    let (width, _) = size(index);
    // Nowhere near its sphere, nowhere near it.
    let offset = placement.position - origin;
    let along = offset.dot(direction);
    if (offset - direction * along).length() > width * 0.5 || along < -width {
        return None;
    }
    let points: Vec<Vec3> = stone(index)
        .iter()
        .map(|point| placement.transform_point(*point))
        .collect();
    stone_triangles()
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
pub const BUDGET: usize = 1_400;

#[cfg(test)]
mod tests;
