// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The backdrop's water, the river, the pool, the waterfall, its foam and mist, and the upper falls and their pool, as
//! triangles the scenery's own shader moves. See the crate's `context/backdrop.md`.
use glam::{Vec2, Vec3};

use super::*;

/// What a point of the water is: its surface, still or flowing, the waterfall, foam, or the mist rising at its foot.
pub const SURFACE: f32 = 0.0;
pub const FALLING: f32 = 1.0;
pub const FOAM: f32 = 2.0;
pub const MIST: f32 = 3.0;

/// A point of the water: where it is, its colour, what it is ([`SURFACE`], [`FALLING`], [`FOAM`] or [`MIST`]), and where it is
/// in its flow, metres across it and along it, downstream or down the fall, which the scenery's shader moves its
/// ripples and streaks along.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Water {
    pub position: Vec3,
    pub color: Vec3,
    pub kind: f32,
    pub flow: Vec2,
}

/// How far the water's surface reaches either side of the river's half width, beneath its banks, so that the shore is
/// where the ground rises out of it.
pub const SPREAD: f32 = 1.8;

/// The water, as triangles: the river from the pool, its surface reaching under its banks; the pool; the waterfall,
/// over the lip and down the cliff in front of its face; and foam and mist about its foot.
pub fn water() -> Vec<Water> {
    let mut triangles = Vec::new();
    let quad = |triangles: &mut Vec<Water>, corners: [Water; 4]| {
        triangles.extend([
            corners[0], corners[1], corners[2], corners[0], corners[2], corners[3],
        ]);
    };
    let point = |position: Vec3, color: Vec3, kind: f32, flow: Vec2| Water {
        position,
        color,
        kind,
        flow,
    };
    // The river.
    let line = layout::river_line(layout::RIVER_STEPS);
    let half = RIVER_HALF_WIDTH + SPREAD;
    let mut along = 0.0;
    let sides: Vec<(Vec3, Vec3, f32)> = line
        .iter()
        .enumerate()
        .map(|(index, middle)| {
            let before = line[index.saturating_sub(1)];
            let after = line[(index + 1).min(line.len() - 1)];
            let tangent = Vec2::new(after.x - before.x, after.z - before.z).normalize();
            let across = Vec3::new(-tangent.y, 0.0, tangent.x) * half;
            if index > 0 {
                along +=
                    Vec2::new(middle.x - line[index - 1].x, middle.z - line[index - 1].z).length();
            }
            (*middle - across, *middle + across, along)
        })
        .collect();
    for pair in sides.windows(2) {
        let [(left, right, from), (next_left, next_right, to)] = [pair[0], pair[1]];
        quad(
            &mut triangles,
            [
                point(left, colors::WATER, SURFACE, Vec2::new(-half, from)),
                point(next_left, colors::WATER, SURFACE, Vec2::new(-half, to)),
                point(next_right, colors::WATER, SURFACE, Vec2::new(half, to)),
                point(right, colors::WATER, SURFACE, Vec2::new(half, from)),
            ],
        );
    }
    // The pool, its ripples spreading from the waterfall's foot.
    let foot = Vec2::new(layout::WATERFALL_FOOT.x, layout::WATERFALL_FOOT.z);
    let rim = POOL_RADIUS + 0.6;
    let middle = POOL + Vec3::Y * 0.02;
    let ripple = |at: Vec3| Vec2::new(0.0, Vec2::new(at.x, at.z).distance(foot));
    for side in 0..12 {
        let corner = |side: u32| {
            let yaw = side as f32 / 12.0 * TAU;
            middle + Vec3::new(yaw.sin(), 0.0, yaw.cos()) * rim
        };
        let (a, b) = (corner(side), corner(side + 1));
        triangles.extend([
            point(middle, colors::WATER, SURFACE, ripple(middle)),
            point(a, colors::WATER, SURFACE, ripple(a)),
            point(b, colors::WATER, SURFACE, ripple(b)),
        ]);
    }
    // The waterfall, from just behind its lip, over it, and down in front of the cliff's face to the pool.
    let outward = way(layout::WATERFALL_AZIMUTH);
    let across = Vec2::new(-outward.y, outward.x);
    let face = |y: f32| {
        let bottom = POOL.y - 0.3;
        layout::CLIFF[0]
            + (layout::CLIFF[1] - layout::CLIFF[0]) * (y - bottom) / (layout::CLIFF_TOP - bottom)
    };
    let top = layout::CLIFF_TOP + 0.08;
    // Over the top, curling out over the lip, where it is palest, and down the face.
    let fall = [
        (layout::CLIFF[1] + 0.9, top),
        (face(layout::CLIFF_TOP) - 0.15, top),
        (face(6.0) - 0.6, 6.0),
        (face(4.6) - 0.5, 4.6),
        (face(3.0) - 0.45, 3.0),
        (face(1.8) - 0.4, 1.8),
        (face(POOL.y) - 0.3, POOL.y + 0.02),
    ];
    let at = |(radius, y): (f32, f32), side: f32| {
        let ground = BACKDROP_CENTRE + outward * radius + across * side;
        Vec3::new(ground.x, y, ground.y)
    };
    const COLUMNS: usize = 4;
    // Narrow where it leaves the lip, fanning out as it falls, its edges ragged, each a little in or out.
    let spread = |row: usize, column: usize| {
        let side = column as f32 / COLUMNS as f32 * 2.0 - 1.0;
        let wide = 0.6 + 0.6 * row as f32 / (fall.len() - 1) as f32;
        let ragged = 1.0 + (scatter((row * 8 + column) as u32 + 1500) - 0.5) * 0.35;
        side * wide * if row == 0 { 1.0 } else { ragged }
    };
    // Paler at the lip and down the middle, darker and thinner at its edges.
    let shade = |row: usize, column: usize| {
        let edge = (column as f32 / COLUMNS as f32 * 2.0 - 1.0).abs();
        let lip = if row == 1 { 1.5 } else { 1.0 };
        colors::FALLING * lip * (1.1 - 0.45 * edge)
    };
    let mut down = 0.0;
    for (row, pair) in fall.windows(2).enumerate() {
        let length = Vec2::new(pair[1].0 - pair[0].0, pair[1].1 - pair[0].1).length();
        for column in 0..COLUMNS {
            let corner = |row: usize, column: usize, from: (f32, f32), down: f32| {
                let lane = column as f32 / COLUMNS as f32 * 2.0 - 1.0;
                point(
                    at(from, spread(row, column)),
                    shade(row, column),
                    FALLING,
                    Vec2::new(lane, down),
                )
            };
            quad(
                &mut triangles,
                [
                    corner(row, column, pair[0], down),
                    corner(row, column + 1, pair[0], down),
                    corner(row + 1, column + 1, pair[1], down + length),
                    corner(row + 1, column, pair[1], down + length),
                ],
            );
        }
        down += length;
    }
    // Foam about its foot, on the pool.
    let splash = at(fall[fall.len() - 1], 0.0) + Vec3::Y * 0.03;
    for side in 0..10 {
        let ring = |side: u32, radius: f32| {
            let yaw = side as f32 / 10.0 * TAU;
            splash + Vec3::new(yaw.sin() * radius * 1.3, 0.0, yaw.cos() * radius)
        };
        let flow = |side: u32, radius: f32| Vec2::new(side as f32 / 10.0 * TAU * radius, radius);
        quad(
            &mut triangles,
            [
                point(ring(side, 0.3), colors::FOAM, FOAM, flow(side, 0.3)),
                point(ring(side + 1, 0.3), colors::FOAM, FOAM, flow(side + 1, 0.3)),
                point(ring(side + 1, 1.9), colors::FOAM, FOAM, flow(side + 1, 1.9)),
                point(ring(side, 1.9), colors::FOAM, FOAM, flow(side, 1.9)),
            ],
        );
    }
    // Mist rising before its foot, in a few low, rounded billows, hiding where it meets the pool.
    for (billow, (side, ahead, radius, height)) in [
        (-0.8, 0.35, 0.75, 0.6),
        (0.1, 0.65, 0.9, 0.8),
        (0.9, 0.3, 0.7, 0.5),
    ]
    .into_iter()
    .enumerate()
    {
        let centre = splash + Vec3::new(across.x, 0.0, across.y) * side
            - Vec3::new(outward.x, 0.0, outward.y) * ahead;
        let corner = |index: u32, level: f32, wide: f32, turn: f32| {
            let yaw = (index as f32 + turn) / 5.0 * TAU + billow as f32;
            let at = centre
                + Vec3::new(
                    yaw.sin() * radius * wide,
                    POOL.y - centre.y + height * level,
                    yaw.cos() * radius * wide,
                );
            point(
                at,
                colors::FOAM * (0.55 + 0.4 * level),
                MIST,
                Vec2::new(yaw * radius, height * level),
            )
        };
        let apex = point(
            centre + Vec3::Y * (POOL.y - centre.y + height),
            colors::FOAM * 0.95,
            MIST,
            Vec2::new(0.0, height),
        );
        for index in 0..5 {
            let (low, next) = (
                corner(index, 0.0, 1.0, 0.0),
                corner(index + 1, 0.0, 1.0, 0.0),
            );
            let (high, beside) = (
                corner(index, 0.55, 0.8, 0.5),
                corner(index + 1, 0.55, 0.8, 0.5),
            );
            triangles.extend([low, next, high, next, beside, high, high, beside, apex]);
        }
    }
    triangles
}

/// The water above the waterfall, drawn with the rest of the water: the upper pool at the upper cliff's foot, its
/// ripples spreading from the upper falls; the stream from its shore to the waterfall's lip, at its level, where the
/// waterfall takes it over; and the upper falls, over the upper cliff's lip and down its face into the pool, foaming at
/// its foot.
pub fn upper_water() -> Vec<Water> {
    let mut triangles = Vec::new();
    let point = |position: Vec3, color: Vec3, kind: f32, flow: Vec2| Water {
        position,
        color,
        kind,
        flow,
    };
    let quad = |triangles: &mut Vec<Water>, corners: [Water; 4]| {
        triangles.extend([
            corners[0], corners[1], corners[2], corners[0], corners[2], corners[3],
        ]);
    };
    let (outward, across) = fall_axes();
    let at = |along: f32, side: f32, y: f32| {
        let ground = BACKDROP_CENTRE + outward * along + across * side;
        Vec3::new(ground.x, y, ground.y)
    };
    // The pool, reaching beneath its shore, its ripples spreading from the falls' foot.
    let pool = layout::UPPER_POOL;
    let foot = Vec2::new(layout::UPPER_FALLS_FOOT.x, layout::UPPER_FALLS_FOOT.z);
    let middle = pool + Vec3::Y * 0.02;
    let ripple = |at: Vec3| Vec2::new(0.0, Vec2::new(at.x, at.z).distance(foot));
    for side in 0..10 {
        let corner = |side: u32| {
            let yaw = side as f32 / 10.0 * TAU;
            middle + Vec3::new(yaw.sin(), 0.0, yaw.cos()) * (layout::UPPER_POOL_RADIUS + 0.3)
        };
        let (a, b) = (corner(side), corner(side + 1));
        triangles.extend([
            point(middle, colors::WATER, SURFACE, ripple(middle)),
            point(a, colors::WATER, SURFACE, ripple(a)),
            point(b, colors::WATER, SURFACE, ripple(b)),
        ]);
    }
    // The stream, from the pool's shore to where the waterfall takes it, just behind its lip, as wide as the
    // waterfall is there, reaching beneath its banks.
    let (head, mouth) = (
        on_the_fall(Vec2::new(pool.x, pool.z)).0 - layout::UPPER_POOL_RADIUS,
        layout::CLIFF[1] + 0.9,
    );
    let reaches: [f32; 4] = [0.0, 0.3, 0.65, 1.0];
    for pair in reaches.windows(2) {
        let (from, to) = (
            head + (mouth - head) * pair[0],
            head + (mouth - head) * pair[1],
        );
        let flow = |along: f32, side: f32| Vec2::new(side, head - along);
        quad(
            &mut triangles,
            [
                point(
                    at(from, -0.6, pool.y),
                    colors::WATER,
                    SURFACE,
                    flow(from, -0.6),
                ),
                point(at(to, -0.6, pool.y), colors::WATER, SURFACE, flow(to, -0.6)),
                point(at(to, 0.6, pool.y), colors::WATER, SURFACE, flow(to, 0.6)),
                point(
                    at(from, 0.6, pool.y),
                    colors::WATER,
                    SURFACE,
                    flow(from, 0.6),
                ),
            ],
        );
    }
    // The upper falls, from just behind the upper cliff's lip, over it and down in front of its face, narrower than
    // the waterfall below, fanning a little as it falls.
    let [low, high] = layout::UPPER_CLIFF;
    let bottom = pool.y - 0.3;
    let face = |y: f32| low + (high - low) * (y - bottom) / (layout::UPPER_TOP - bottom);
    let top = layout::UPPER_TOP + 0.08;
    let fall = [
        (high + 0.6, top),
        (face(layout::UPPER_TOP) - 0.12, top),
        (face(8.8) - 0.4, 8.8),
        (face(7.7) - 0.35, 7.7),
        (face(pool.y) - 0.3, pool.y + 0.02),
    ];
    const COLUMNS: usize = 3;
    let spread = |row: usize, column: usize| {
        let side = column as f32 / COLUMNS as f32 * 2.0 - 1.0;
        let wide = 0.3 + 0.2 * row as f32 / (fall.len() - 1) as f32;
        let ragged = 1.0 + (scatter((row * 8 + column) as u32 + 1700) - 0.5) * 0.3;
        side * wide * if row == 0 { 1.0 } else { ragged }
    };
    let shade = |row: usize, column: usize| {
        let edge = (column as f32 / COLUMNS as f32 * 2.0 - 1.0).abs();
        let lip = if row == 1 { 1.5 } else { 1.0 };
        colors::FALLING * lip * (1.1 - 0.45 * edge)
    };
    let mut down = 0.0;
    for (row, pair) in fall.windows(2).enumerate() {
        let length = Vec2::new(pair[1].0 - pair[0].0, pair[1].1 - pair[0].1).length();
        for column in 0..COLUMNS {
            let corner = |row: usize, column: usize, (along, y): (f32, f32), down: f32| {
                let lane = column as f32 / COLUMNS as f32 * 2.0 - 1.0;
                point(
                    at(along, spread(row, column), y),
                    shade(row, column),
                    FALLING,
                    Vec2::new(lane, down),
                )
            };
            quad(
                &mut triangles,
                [
                    corner(row, column, pair[0], down),
                    corner(row, column + 1, pair[0], down),
                    corner(row + 1, column + 1, pair[1], down + length),
                    corner(row + 1, column, pair[1], down + length),
                ],
            );
        }
        down += length;
    }
    // Foam about its foot, on the pool.
    let (along, _) = fall[fall.len() - 1];
    let splash = at(along, 0.0, pool.y + 0.05);
    for side in 0..8 {
        let ring = |side: u32, radius: f32| {
            let yaw = side as f32 / 8.0 * TAU;
            splash + Vec3::new(yaw.sin() * radius, 0.0, yaw.cos() * radius)
        };
        let flow = |side: u32, radius: f32| Vec2::new(side as f32 / 8.0 * TAU * radius, radius);
        quad(
            &mut triangles,
            [
                point(ring(side, 0.12), colors::FOAM, FOAM, flow(side, 0.12)),
                point(
                    ring(side + 1, 0.12),
                    colors::FOAM,
                    FOAM,
                    flow(side + 1, 0.12),
                ),
                point(ring(side + 1, 0.6), colors::FOAM, FOAM, flow(side + 1, 0.6)),
                point(ring(side, 0.6), colors::FOAM, FOAM, flow(side, 0.6)),
            ],
        );
    }
    triangles
}
