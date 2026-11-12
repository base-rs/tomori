// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The ranges beyond the foothills, directions rather than places, which the scenery draws at the sky's depth, each
//! faded into the sky's colour that way. See the crate's `context/backdrop.md`.
use glam::Vec3;

use super::colors;
use crate::props::{evening, scatter};

/// A point of the ranges, far beyond the backdrop's 50 m bound: which way it is, its colour lit by the moon, and how far
/// it fades into the sky's colour that way, for the haze of its distance.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Far {
    pub direction: Vec3,
    pub color: Vec3,
    pub fade: f32,
}

/// One range of mountains: how many points its ridge has, how high it rises, in degrees above the horizon, at least and
/// how much more, its peaks, as `(azimuth, height, width)` in degrees, above which elevation its ridge is snowy, how far
/// down its upper facets reach, its rock's colour, and how far it fades into the sky.
pub(super) struct Range {
    pub(super) points: usize,
    pub(super) base: f32,
    pub(super) relief: f32,
    pub(super) peaks: &'static [(f32, f32, f32)],
    pub(super) snow: f32,
    pub(super) shoulder: f32,
    pub(super) rock: Vec3,
    pub(super) fade: f32,
    pub(super) seed: u32,
}

/// The ranges, from the farthest, drawn first, to the nearest: the snowy alps, highest behind the waterfall and under
/// the moon; a bluer range before them; and a bush-clad range beyond the foothills.
pub(super) const RANGES: [Range; 3] = [
    Range {
        points: 128,
        base: 5.0,
        relief: 5.0,
        peaks: &[
            (-12.0, 10.0, 9.0),
            (8.0, 5.0, 7.0),
            (40.0, 7.0, 10.0),
            (-60.0, 4.5, 8.0),
            (110.0, 4.0, 9.0),
            (175.0, 3.5, 12.0),
            (-125.0, 5.0, 10.0),
        ],
        snow: 9.5,
        shoulder: 0.24,
        rock: colors::RANGE_FAR,
        fade: 0.55,
        seed: 900,
    },
    Range {
        points: 96,
        base: 3.5,
        relief: 4.5,
        peaks: &[(25.0, 2.0, 12.0), (-95.0, 2.5, 14.0), (150.0, 2.0, 10.0)],
        snow: 10.5,
        shoulder: 0.45,
        rock: colors::RANGE_MIDDLE,
        fade: 0.45,
        seed: 1900,
    },
    Range {
        points: 72,
        base: 2.0,
        relief: 3.5,
        peaks: &[],
        snow: 90.0,
        shoulder: 0.55,
        rock: colors::RANGE_NEAR,
        fade: 0.3,
        seed: 2900,
    },
];

/// How far below the horizon the ranges reach, in degrees, so that no gap shows beneath them.
pub(super) const FOOT: f32 = -3.0;

/// The colour of a face of `albedo` facing `normal`, lit as the farmyard is: the sky's and the ground's light, and the
/// moon's.
fn lit(albedo: Vec3, normal: Vec3) -> Vec3 {
    let ambient = evening::GROUND.lerp(evening::SKY, normal.y * 0.5 + 0.5);
    albedo * (ambient + evening::MOONLIGHT * normal.dot(evening::MOON.normalize()).max(0.0))
}

/// The direction `elevation` degrees above the horizon at `azimuth` degrees from −Z towards +X.
pub(super) fn towards(azimuth: f32, elevation: f32) -> Vec3 {
    let (azimuth, elevation) = (azimuth.to_radians(), elevation.to_radians());
    Vec3::new(
        azimuth.sin() * elevation.cos(),
        elevation.sin(),
        -azimuth.cos() * elevation.cos(),
    )
}

/// The ranges, as triangles of directions, from the farthest to the nearest, each drawn over the last: each ridge's
/// point, a shoulder below it and its foot below the horizon, faceted so that the moon lights some faces and not
/// others, the highest peaks' upper faces snowy.
pub fn ranges() -> Vec<Far> {
    let mut triangles = Vec::new();
    for range in &RANGES {
        let knots = range.points / 8;
        // A smooth roll through knots, then a little roughness at every point, then the peaks.
        let roll = |at: f32| {
            let knot = at * knots as f32;
            let (index, t) = (knot.floor() as usize, knot.fract());
            let value = |k: usize| scatter(range.seed + (k % knots) as u32);
            value(index) + (value(index + 1) - value(index)) * t * t * (3.0 - 2.0 * t)
        };
        let points: Vec<(f32, f32, f32)> = (0..range.points)
            .map(|point| {
                let at = point as f32 / range.points as f32;
                let azimuth = at * 360.0;
                let mut elevation = range.base
                    + range.relief
                        * (0.75 * roll(at) + 0.25 * scatter(range.seed + 500 + point as u32));
                for (middle, height, width) in range.peaks {
                    let apart = ((azimuth - middle + 540.0) % 360.0 - 180.0).abs();
                    elevation += height * (1.0 - apart / width).max(0.0);
                }
                let step = 360.0 / range.points as f32;
                let shoulder =
                    azimuth + (scatter(range.seed + 1000 + point as u32) - 0.5) * step * 0.6;
                (azimuth, elevation, shoulder)
            })
            .collect();
        for point in 0..range.points {
            let (azimuth, elevation, shoulder) = points[point];
            let (next, next_elevation, next_shoulder) = points[(point + 1) % range.points];
            let next = if next < azimuth { next + 360.0 } else { next };
            let next_shoulder = if next_shoulder < shoulder - 180.0 {
                next_shoulder + 360.0
            } else {
                next_shoulder
            };
            let below = |elevation: f32| elevation - (elevation - FOOT) * range.shoulder;
            // Each corner as a direction and a place, nearer the lower it is, so that the facets lean back.
            let corner = |azimuth: f32, elevation: f32, distance: f32| {
                (
                    towards(azimuth, elevation),
                    towards(azimuth, elevation) * distance,
                )
            };
            let ridge = [
                corner(azimuth, elevation, 1.0),
                corner(next, next_elevation, 1.0),
            ];
            let shoulders = [
                corner(shoulder, below(elevation), 0.8),
                corner(next_shoulder, below(next_elevation), 0.8),
            ];
            let feet = [corner(azimuth, FOOT, 0.65), corner(next, FOOT, 0.65)];
            let snowy = elevation.max(next_elevation) >= range.snow;
            let faces = if point % 2 == 0 {
                [
                    ([ridge[0], ridge[1], shoulders[0]], snowy),
                    ([shoulders[0], ridge[1], shoulders[1]], snowy),
                    ([shoulders[0], shoulders[1], feet[0]], false),
                    ([feet[0], shoulders[1], feet[1]], false),
                ]
            } else {
                [
                    ([ridge[0], ridge[1], shoulders[1]], snowy),
                    ([ridge[0], shoulders[1], shoulders[0]], snowy),
                    ([shoulders[0], shoulders[1], feet[1]], false),
                    ([shoulders[0], feet[1], feet[0]], false),
                ]
            };
            for (index, (corners, snow)) in faces.into_iter().enumerate() {
                let places = corners.map(|(_, place)| place);
                // Facing the eye, whichever way the corners wind.
                let normal = (places[1] - places[0])
                    .cross(places[2] - places[0])
                    .normalize();
                let normal = normal * -normal.dot(places[0]).signum();
                let albedo = if snow { colors::SNOW } else { range.rock };
                let varied = 0.9 + scatter(range.seed + 3000 + (point * 4 + index) as u32) * 0.2;
                let color = lit(albedo * varied, normal);
                triangles.extend(corners.map(|(direction, _)| Far {
                    direction,
                    color,
                    fade: range.fade,
                }));
            }
        }
    }
    triangles
}
