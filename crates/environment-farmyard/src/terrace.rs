// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The upper terrace: a private place at the top of the waterfall, on the mountain behind it, reached only by pointing
//! at the falling water from the waterfall rock, and left by pointing at the cairn by the lip, which takes one back onto
//! the rock. A floor of moss and stone 7.2 m above the pool, beside the stream that runs from the upper pool to the
//! waterfall's lip, before the upper falls, which drop from the upper cliff behind it into that pool; a flat rock to sit
//! on, looking out over the yard; stones along its edges, ferns at its back, and the cairn.
//!
//! The floor is drawn as ground ([`ground`]), which the farmyard's walkable ground is derived from with the rest of it,
//! less what stands on it ([`solids`]), so that it is a walkable region of its own at its height, joined to nothing
//! else: nobody walks or steps up to it, and its edge stops short of the lip. Its anchor, `upper-terrace`, is beside
//! whoever arrives there ([`place`]), and its two teleport targets are the way up and the way down ([`targets`]). Where
//! everything stands is said along the waterfall's way from the backdrop's middle and across it ([`on_fall`]). See the
//! crate's `context/terrace.md`.
use std::f32::consts::TAU;

use glam::{Quat, Vec2, Vec3};
use tomori_assets::Model;

use crate::backdrop::{self, Stone, colors};
use crate::ferns::{Fern, Kind};
use crate::layout::{self, TERRACE};
use crate::props::scatter;
use crate::shapes::Shapes;
use crate::trail::{Place, TargetPlace};

/// The point `along` the waterfall's way from the backdrop's middle and `across` it, to the right as one faces the fall
/// from the yard, on the ground.
pub fn on_fall(along: f32, across: f32) -> Vec2 {
    let (outward, right) = backdrop::fall_axes();
    layout::BACKDROP_CENTRE + outward * along + right * across
}

/// [`on_fall`], at `height`.
fn at(along: f32, across: f32, height: f32) -> Vec3 {
    let point = on_fall(along, across);
    Vec3::new(point.x, height, point.y)
}

/// The floor's outline, each corner along the waterfall's way and across it, anticlockwise seen from above: along the
/// cliff's top behind the lip, a little back from where the cliff falls away, so that its edge stops short of it; up the
/// mountain's right side; along the foot of the upper cliff behind it, short of its rise; and down the stream's bank,
/// past the upper pool's shore, to the lip.
pub const OUTLINE: [(f32, f32); 14] = [
    (24.2, 2.1),
    (24.1, 3.3),
    (25.05, 4.9),
    (24.75, 6.5),
    (24.2, 8.1),
    (24.1, 9.2),
    (24.6, 10.4),
    (24.75, 11.4),
    (26.9, 11.4),
    (27.4, 9.6),
    (28.0, 7.5),
    (28.7, 4.5),
    (29.0, 2.4),
    (28.6, 1.9),
];

/// The floor's outline, on the ground.
pub fn outline() -> Vec<Vec2> {
    OUTLINE
        .iter()
        .map(|(along, across)| on_fall(*along, *across))
        .collect()
}

/// How far the floor's sides reach down below it, into the mountain, so that no gap shows beneath its edge where the
/// cliff's top falls away a little sooner.
pub const SIDES: f32 = 0.8;

/// Whether `point`, on the ground, is within the floor's outline.
pub fn contains(point: Vec2) -> bool {
    let outline = outline();
    let mut inside = false;
    for (index, corner) in outline.iter().enumerate() {
        let next = outline[(index + 1) % outline.len()];
        if (corner.y > point.y) != (next.y > point.y)
            && point.x < corner.x + (point.y - corner.y) / (next.y - corner.y) * (next.x - corner.x)
        {
            inside = !inside;
        }
    }
    inside
}

/// How far `point`, on the ground, is from the floor: none within it.
pub fn from_floor(point: Vec2) -> f32 {
    if contains(point) {
        return 0.0;
    }
    from_outline(&outline(), point)
}

/// How far `point`, on the ground, is from the nearest edge of `outline`, within it or beyond.
fn from_outline(outline: &[Vec2], point: Vec2) -> f32 {
    (0..outline.len())
        .map(|index| {
            let (from, to) = (outline[index], outline[(index + 1) % outline.len()]);
            let along = to - from;
            let t = ((point - from).dot(along) / along.length_squared()).clamp(0.0, 1.0);
            point.distance(from + along * t)
        })
        .fold(f32::INFINITY, f32::min)
}

/// How far apart the floor's corners are, about: its outline's edges are cut into pieces no longer, and its corners
/// within lie on a lattice of even triangles this far apart, along the waterfall's way and across it, so that its faces
/// are even in size and shape, with no corner that many meet at.
pub const SPACING: f32 = 0.6;

/// How far from its edge, by [`SPACING`], the lattice's corners within the floor stay, so that none crowds a corner of
/// the edge into a sliver.
const MARGIN: f32 = 0.55;

/// The floor's edge: its outline, each edge cut evenly into pieces of [`SPACING`] or less, on the ground, in order,
/// which its faces and its sides share, so that they meet corner to corner.
pub fn edge() -> Vec<Vec2> {
    let outline = outline();
    (0..outline.len())
        .flat_map(|index| {
            let (from, to) = (outline[index], outline[(index + 1) % outline.len()]);
            let pieces = (from.distance(to) / SPACING).ceil().max(1.0) as usize;
            (0..pieces).map(move |piece| from.lerp(to, piece as f32 / pieces as f32))
        })
        .collect()
}

/// The floor's faces, on the ground: a constrained Delaunay triangulation
/// of its edge ([`edge`]) and of the corners of a lattice of even triangles [`SPACING`] apart within it, refined so that
/// no angle of any face is less than 25°, its edge's pieces kept whole.
pub fn faces() -> Vec<[Vec2; 3]> {
    use spade::{
        AngleLimit, ConstrainedDelaunayTriangulation, Point2, RefinementParameters, Triangulation,
    };
    let edge = edge();
    let outline = outline();
    // The lattice, its rows across the waterfall's way, each a half step along from the one before.
    let (low, high) = OUTLINE.iter().fold(
        (Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY)),
        |(low, high), (along, across)| {
            let at = Vec2::new(*along, *across);
            (low.min(at), high.max(at))
        },
    );
    let row = SPACING * 3f32.sqrt() * 0.5;
    let rows = ((high.x - low.x) / row).ceil() as usize;
    let columns = ((high.y - low.y) / SPACING).ceil() as usize + 1;
    let within = (0..=rows)
        .flat_map(|step| (0..=columns).map(move |column| (step, column)))
        .map(|(step, column)| {
            let shift = if step % 2 == 0 { 0.0 } else { 0.5 };
            on_fall(
                low.x + step as f32 * row,
                low.y + (column as f32 + shift) * SPACING,
            )
        })
        .filter(|point| contains(*point) && from_outline(&outline, *point) > SPACING * MARGIN);
    let points: Vec<Point2<f64>> = edge
        .iter()
        .copied()
        .chain(within)
        .map(|point| Point2::new(point.x as f64, point.y as f64))
        .collect();
    let constraints = (0..edge.len())
        .map(|index| [index, (index + 1) % edge.len()])
        .collect();
    let mut triangulation: ConstrainedDelaunayTriangulation<Point2<f64>> =
        ConstrainedDelaunayTriangulation::bulk_load_cdt(points, constraints)
            .expect("the floor's corners are finite and its edge does not cross itself");
    let refined = triangulation.refine(
        RefinementParameters::new()
            .with_angle_limit(AngleLimit::from_deg(25.0))
            .keep_constraint_edges()
            .exclude_outer_faces(true),
    );
    let outer: std::collections::HashSet<_> = refined.excluded_faces.into_iter().collect();
    triangulation
        .inner_faces()
        .filter(|face| !outer.contains(&face.fix()))
        .map(|face| {
            face.positions()
                .map(|point| Vec2::new(point.x as f32, point.y as f32))
        })
        .collect()
}

/// The shade of the floor about `point`, on the ground: moss, but in patches of stone where its moss is worn away.
fn shade(point: Vec2) -> Vec3 {
    let wave =
        (point.x * 1.3 + point.y * 0.7).sin() + (point.y * 1.1 - point.x * 0.8 + 1.9).sin() * 0.8;
    if wave > 1.0 {
        paints::STONE
    } else {
        paints::MOSS
    }
}

/// The colours of the terrace: its floor, moss and stone, and its flat rock and cairn, paler where worn.
pub mod paints {
    use glam::Vec3;

    pub const MOSS: Vec3 = Vec3::new(0.085, 0.135, 0.06);
    pub const STONE: Vec3 = Vec3::new(0.19, 0.185, 0.17);
    pub const WORN: Vec3 = Vec3::new(0.3, 0.29, 0.27);
}

/// The flat rock to sit on, looking out over the yard: its middle, on the floor, how far it reaches along its length
/// and across, and how high its top is above the floor.
pub const SEAT: (f32, f32) = (25.0, 7.1);
pub const SEAT_SIZE: Vec3 = Vec3::new(0.5, 0.42, 0.28);

/// The cairn by the waterfall's lip, the way back down: its middle, on the floor, along the waterfall's way and across
/// it, how far its widest stone reaches and how tall it stands.
pub const CAIRN: (f32, f32) = (24.65, 2.65);
pub const CAIRN_REACH: f32 = 0.3;
pub const CAIRN_HEIGHT: f32 = 0.62;

/// The cairn's stones, from the bottom, each its radius and how thick it is.
const CAIRN_STONES: [(f32, f32); 4] = [(0.3, 0.17), (0.24, 0.16), (0.18, 0.15), (0.12, 0.14)];

/// Where a participant arrives on the terrace, pointing at the waterfall from its rock, and which way they face there:
/// the upper falls.
pub const ARRIVAL: (f32, f32) = (26.4, 4.6);

/// The floor, the flat rock and the cairn's shapes.
fn shapes() -> Shapes {
    let mut shapes = Shapes::new();
    let edge = edge();
    let lift = |point: Vec2, height: f32| Vec3::new(point.x, height, point.y);
    // The floor, in even faces, each moss or stone as the floor is about it, a little lighter or darker than the next.
    shapes.ground(|shapes| {
        for (index, [a, b, c]) in faces().into_iter().enumerate() {
            let color = shade((a + b + c) / 3.0);
            backdrop::facing(
                shapes,
                &[lift(a, TERRACE), lift(b, TERRACE), lift(c, TERRACE)],
                Vec3::Y,
                color * (0.94 + scatter(index as u32 + 18_000) * 0.12),
            );
        }
    });
    // Its sides, rock, down into the mountain, from each piece of its edge, which they share with the floor.
    for (index, corner) in edge.iter().enumerate() {
        let next = edge[(index + 1) % edge.len()];
        let outward = (next - *corner).perp() * -1.0;
        let outward = Vec3::new(outward.x, 0.0, outward.y);
        backdrop::facing(
            &mut shapes,
            &[
                lift(*corner, TERRACE),
                lift(next, TERRACE),
                lift(next, TERRACE - SIDES),
                lift(*corner, TERRACE - SIDES),
            ],
            outward,
            colors::ROCK * (0.9 + scatter(index as u32 + 18_100) * 0.2),
        );
    }
    // The flat rock, a low slab of six sides, its top worn paler.
    let (along, across) = SEAT;
    let (outward, right) = backdrop::fall_axes();
    let seat = on_fall(along, across);
    let corner = |side: u32, reach: f32, height: f32| {
        let yaw = side as f32 / 6.0 * TAU + 0.2;
        let (cos, sin) = (yaw.cos(), yaw.sin());
        let point = seat
            + right * cos * SEAT_SIZE.x * reach * (0.9 + scatter(side + 18_200) * 0.1)
            + outward * sin * SEAT_SIZE.z * reach;
        lift(point, TERRACE + height)
    };
    // Turning from across the fall towards the yard, anticlockwise seen from above, as a slab is drawn.
    let base: Vec<Vec3> = (0..6).map(|side| corner(side, 1.0, -0.05)).collect();
    let top: Vec<Vec3> = (0..6).map(|side| corner(side, 0.82, SEAT_SIZE.y)).collect();
    shapes.slab(&base, &top, paints::WORN);
    // The cairn: four flat stones, each smaller than the one below it, each turned its own way, anticlockwise seen from
    // above.
    let (along, across) = CAIRN;
    let mut level = TERRACE - 0.03;
    for (index, (reach, thick)) in CAIRN_STONES.iter().enumerate() {
        let middle = at(along, across, level);
        let turn = scatter(index as u32 + 18_300) * TAU;
        let corner = |side: u32, reach: f32, height: f32| {
            let yaw = side as f32 / 7.0 * TAU + turn;
            middle + Quat::from_rotation_y(yaw) * Vec3::Z * reach + Vec3::Y * height
        };
        let base: Vec<Vec3> = (0..7).map(|side| corner(side, *reach, 0.0)).collect();
        let top: Vec<Vec3> = (0..7)
            .map(|side| corner(side, reach * 0.8, *thick))
            .collect();
        let shade = 0.9 + scatter(index as u32 + 18_310) * 0.25;
        shapes.slab(&base, &top, paints::WORN * shade);
        level += thick - 0.01;
    }
    shapes
}

/// The terrace, as a model of one node whose mesh draws it all: its floor and its sides, the flat rock and the cairn.
/// Its stones are the river's, drawn with them ([`stones`]), and its ferns are drawn with the ferns ([`ferns`]).
pub fn model() -> Model {
    backdrop::built("terrace", shapes())
}

/// What the terrace draws as ground, which participants walk on: its floor's top.
pub fn ground() -> Vec<[Vec3; 3]> {
    shapes().walkable().to_vec()
}

/// How high the cairn's top is.
pub fn cairn_top() -> f32 {
    TERRACE - 0.03
        + CAIRN_STONES
            .iter()
            .map(|(_, thick)| thick - 0.01)
            .sum::<f32>()
        + 0.01
}

/// The stones at the terrace's edges, the river's own model, grey and rounded, sitting on its floor or beside it: a
/// few along the lip, at the foot of the upper cliff and on the stream's bank, the same every time.
pub fn stones() -> Vec<Stone> {
    const PLACES: [(f32, f32, f32); 8] = [
        (24.15, 4.1, 0.36),
        (24.9, 5.75, 0.34),
        (24.4, 7.6, 0.4),
        (24.05, 8.85, 0.32),
        (24.4, 10.05, 0.38),
        (28.95, 5.6, 0.5),
        (27.75, 9.9, 0.46),
        (26.0, 1.75, 0.32),
    ];
    PLACES
        .iter()
        .enumerate()
        .map(|(index, (along, across, size))| {
            let seed = index as u32 * 7 + 18_400;
            Stone {
                at: at(*along, *across, TERRACE - 0.03),
                turn: scatter(seed) * TAU,
                size: Vec2::new(*size, size * (0.75 + scatter(seed + 1) * 0.35)),
            }
        })
        .collect()
}

/// The terrace's ferns, drawn with the ferns near the trail and swaying with them: a ponga at its back corner, beneath
/// the upper cliff, its crown high over the floor, a mamaku across the stream from it, over the waterfall's lip, and
/// ground ferns at its front corner, by the stream and at its back, each placed by hand, at its edges, since the
/// terrace is small, and out of the way from where one arrives to the upper falls.
pub fn ferns() -> Vec<Fern> {
    const PLACES: [(Kind, f32, f32, f32); 5] = [
        (Kind::Ponga, 27.2, 10.9, 0.95),
        (Kind::Mamaku, 25.5, -1.9, 0.9),
        (Kind::CrownFern, 24.6, 11.1, 0.85),
        (Kind::HenAndChickens, 26.9, 2.25, 0.8),
        (Kind::Kiokio, 28.4, 6.4, 0.75),
    ];
    PLACES
        .iter()
        .enumerate()
        .map(|(index, (kind, along, across, scale))| {
            let seed = index as u32 * 5 + 18_500;
            Fern {
                kind: *kind,
                at: at(*along, *across, TERRACE - 0.02),
                turn: scatter(seed) * TAU,
                scale: *scale,
                phase: scatter(seed + 1) * TAU,
            }
        })
        .collect()
}

/// The terrace's solids, which participants keep clear of and the agent, walls and panels keep out of, as `(least,
/// greatest)` corners: its stones, the flat rock, the cairn and its ferns, the tree ferns' trunks and crowns and the
/// ground ferns' clumps.
pub fn solids() -> Vec<([f32; 3], [f32; 3])> {
    let mut solids: Vec<([f32; 3], [f32; 3])> = stones()
        .iter()
        .map(|stone| {
            let solid = stone.solid();
            (solid.low, solid.high)
        })
        .collect();
    let (along, across) = SEAT;
    let seat = at(along, across, TERRACE);
    let reach = SEAT_SIZE.x.max(SEAT_SIZE.z) * 1.05;
    solids.push((
        [seat.x - reach, TERRACE, seat.z - reach],
        [seat.x + reach, TERRACE + SEAT_SIZE.y, seat.z + reach],
    ));
    let (along, across) = CAIRN;
    let cairn = at(along, across, TERRACE);
    solids.push((
        [cairn.x - CAIRN_REACH, TERRACE, cairn.z - CAIRN_REACH],
        [cairn.x + CAIRN_REACH, cairn_top(), cairn.z + CAIRN_REACH],
    ));
    solids.extend(
        ferns()
            .iter()
            .flat_map(Fern::solids)
            .map(|solid| (solid.low, solid.high)),
    );
    solids
}

/// How far from whoever stands where the terrace takes them, back from the upper falls they face, its anchor stands,
/// and how high above the floor: behind them, so that whatever gathers there is beside them rather than before the
/// falls, as the waterfall rock's is.
const ANCHOR: (f32, f32) = (1.6, 1.8);

/// Where participants arrive on the terrace, on its floor.
pub fn arrival() -> Vec3 {
    let (along, across) = ARRIVAL;
    at(along, across, TERRACE)
}

/// The terrace's named anchor, `upper-terrace`, a viewpoint, behind whoever arrives there, facing them, so that the
/// agent may go there, and workers be placed there, in the trail's room.
pub fn place() -> Place {
    let arrival = arrival();
    let facing = level(layout::UPPER_FALLS, arrival).normalize();
    let (behind, high) = ANCHOR;
    let at = arrival - facing * behind + Vec3::Y * high;
    Place {
        name: "upper-terrace",
        kind: "viewpoint",
        label: "The upper terrace",
        at,
        towards: arrival + Vec3::Y * high,
        notify_within: None,
    }
}

/// `towards`, seen from `from`, horizontally.
fn level(towards: Vec3, from: Vec3) -> Vec3 {
    (towards - from) * Vec3::new(1.0, 0.0, 1.0)
}

/// How far from the waterfall's foot the rock's top is reached from, the way up: the rock's top, where one stands
/// pointing at the fall, and nowhere else participants stand, so that the waterfall takes them up only from its rock.
pub const FALL_REACH: f32 = 5.0;
/// How far from the cairn's foot one points at it, the way down: from the terrace alone.
pub const CAIRN_POINTED: f32 = 4.5;

/// The terrace's teleport targets: the waterfall, pointed at from its rock within [`FALL_REACH`], taking one up onto
/// the terrace, facing the upper falls; and the cairn by the lip, pointed at from the terrace within
/// [`CAIRN_POINTED`], taking one back down onto the rock's top, facing the waterfall.
pub fn targets() -> [TargetPlace; 2] {
    // The fall, about the lower part of its sheet, before the cliff's face, as tall as a target may be.
    let fall = on_fall(layout::CLIFF[0] + 0.1, 0.0);
    let rock = layout::WATERFALL_ROCK;
    let arrival = arrival();
    let (along, across) = CAIRN;
    [
        TargetPlace {
            foot: Vec3::new(fall.x, layout::POOL.y, fall.y),
            radius: 1.0,
            height: FALL_HEIGHT,
            arrival,
            name: "Upper terrace",
            facing: level(layout::UPPER_FALLS, arrival),
            within: Some(FALL_REACH),
        },
        TargetPlace {
            foot: at(along, across, TERRACE),
            radius: CAIRN_REACH + 0.05,
            height: cairn_top() - TERRACE + 0.05,
            arrival: rock,
            name: "Waterfall rock",
            facing: level(layout::WATERFALL, rock),
            within: Some(CAIRN_POINTED),
        },
    ]
}

/// How tall the waterfall is pointed at, from the pool: the lower part of its fall, as tall as a teleport target may
/// be.
pub const FALL_HEIGHT: f32 = 4.0;

#[cfg(test)]
mod tests;
