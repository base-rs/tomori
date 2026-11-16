// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Noise that tiles: every function here takes a point of the tile, `u` and `v` from 0 to 1, and a lattice of a whole
//! number of cells across and down it, whose corners wrap, so that what is at `u + 1` is exactly what is at `u`. It is
//! made of integer hashes and arithmetic alone, the same to the bit on every machine.

/// A number made from a lattice point and a seed, scrambled so that neighbours share nothing.
pub fn hash(x: u32, y: u32, seed: u32) -> u32 {
    let mut hash =
        x.wrapping_mul(0x8da6_b343) ^ y.wrapping_mul(0xd816_3841) ^ seed.wrapping_mul(0xcb1a_b31f);
    hash ^= hash >> 16;
    hash = hash.wrapping_mul(0x7feb_352d);
    hash ^= hash >> 15;
    hash = hash.wrapping_mul(0x846c_a68b);
    hash ^ hash >> 16
}

/// A number from 0 up to 1 made from `hash`, exactly: its top 24 bits.
pub fn unit(hash: u32) -> f32 {
    (hash >> 8) as f32 * (1.0 / 16_777_216.0)
}

/// Where a coordinate lies on a lattice of `cells` that wraps: the cell it is in, the next, and how far across it is.
fn cell(at: f32, cells: u32) -> (u32, u32, f32) {
    let scaled = at * cells as f32;
    let floor = scaled.floor();
    let index = (floor as i64).rem_euclid(i64::from(cells)) as u32;
    (index, (index + 1) % cells, scaled - floor)
}

/// Quintic easing, so that noise is smooth in its slope as well as its value across a cell's edges.
fn fade(t: f32) -> f32 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Value noise from 0 up to 1 at `(u, v)` of the tile, on a lattice of `across` by `down` cells.
pub fn value(u: f32, v: f32, (across, down): (u32, u32), seed: u32) -> f32 {
    let (x0, x1, tx) = cell(u, across);
    let (y0, y1, ty) = cell(v, down);
    let corner = |x, y| unit(hash(x, y, seed));
    let (tx, ty) = (fade(tx), fade(ty));
    lerp(
        lerp(corner(x0, y0), corner(x1, y0), tx),
        lerp(corner(x0, y1), corner(x1, y1), tx),
        ty,
    )
}

/// Fractal noise from 0 to about 1: `octaves` of [`value`] noise, the first on `cells`, each next twice as fine and
/// `gain` as strong, their sum divided by their strengths' so that its mean is a half.
pub fn fractal(u: f32, v: f32, cells: (u32, u32), octaves: u32, gain: f32, seed: u32) -> f32 {
    let (mut sum, mut total, mut strength) = (0.0, 0.0, 1.0);
    for octave in 0..octaves {
        let scale = 1 << octave;
        sum += strength
            * value(
                u,
                v,
                (cells.0 * scale, cells.1 * scale),
                seed.wrapping_add(octave * 7919),
            );
        total += strength;
        strength *= gain;
    }
    sum / total
}

/// Ridges along the middle of fractal noise: 1 where it crosses a half, falling to nothing at its extremes.
pub fn ridged(u: f32, v: f32, cells: (u32, u32), octaves: u32, gain: f32, seed: u32) -> f32 {
    1.0 - (2.0 * fractal(u, v, cells, octaves, gain, seed) - 1.0).abs()
}

/// The nearest of the points scattered one in each cell of a lattice that wraps, about `(u, v)`, looked for two cells
/// about it each way, since a point two cells away may be nearer than the one in its own cell.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cells {
    /// How far the nearest point is, and the next nearest, in cells.
    pub nearest: f32,
    pub second: f32,
    /// The nearest point's hash, which its cell's features are made from.
    pub id: u32,
    /// How far the nearest point is across and down from `(u, v)`, in cells.
    pub offset: (f32, f32),
}

/// The points of a lattice of `across` by `down` cells, one at a place of its cell its hash chooses, nearest `(u, v)`,
/// measured in cells, so that a lattice of fewer cells down than across makes cells longer than they are wide.
pub fn cells(u: f32, v: f32, (across, down): (u32, u32), seed: u32) -> Cells {
    let (x, _, fx) = cell(u, across);
    let (y, _, fy) = cell(v, down);
    let mut found = Cells {
        nearest: f32::MAX,
        second: f32::MAX,
        id: 0,
        offset: (0.0, 0.0),
    };
    for dy in -2i64..=2 {
        for dx in -2i64..=2 {
            let cx = (i64::from(x) + dx).rem_euclid(i64::from(across)) as u32;
            let cy = (i64::from(y) + dy).rem_euclid(i64::from(down)) as u32;
            let id = hash(cx, cy, seed);
            let px = dx as f32 + unit(id) - fx;
            let py = dy as f32 + unit(hash(id, cy, seed ^ 0x5bd1_e995)) - fy;
            let distance = (px * px + py * py).sqrt();
            if distance < found.nearest {
                found.second = found.nearest;
                found = Cells {
                    nearest: distance,
                    id,
                    offset: (px, py),
                    ..found
                };
            } else if distance < found.second {
                found.second = distance;
            }
        }
    }
    found
}

/// Hermite's smooth step from `edge0` to `edge1`, either way round.
pub fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noise_wraps_exactly_and_stays_within_its_range() {
        for step in 0..200 {
            let (u, v) = (step as f32 * 0.013 + 0.001, step as f32 * 0.029 + 0.002);
            for (a, b) in [
                (value(u, v, (6, 4), 3), value(u + 1.0, v - 1.0, (6, 4), 3)),
                (
                    fractal(u, v, (3, 5), 5, 0.5, 9),
                    fractal(u - 1.0, v + 2.0, (3, 5), 5, 0.5, 9),
                ),
                (
                    ridged(u, v, (4, 4), 3, 0.6, 1),
                    ridged(u + 3.0, v, (4, 4), 3, 0.6, 1),
                ),
            ] {
                assert!((a - b).abs() < 1e-5, "{a} {b}");
                assert!((0.0..=1.0).contains(&a));
            }
            let (here, there) = (cells(u, v, (8, 5), 2), cells(u + 1.0, v + 1.0, (8, 5), 2));
            assert_eq!(here.id, there.id);
            assert!((here.nearest - there.nearest).abs() < 1e-4);
            assert!(here.nearest <= here.second);
            assert!(here.nearest <= 2f32.sqrt() + 1e-4);
        }
        // Seeds and lattice points give unrelated numbers.
        assert_ne!(hash(1, 2, 3), hash(2, 1, 3));
        assert_ne!(hash(1, 2, 3), hash(1, 2, 4));
        assert_eq!(unit(u32::MAX), 16_777_215.0 / 16_777_216.0);
        assert_eq!(smoothstep(1.0, 0.0, 0.25), 1.0 - smoothstep(0.0, 1.0, 0.25));
    }
}
