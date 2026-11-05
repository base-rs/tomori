// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! A signed distance field from antialiased coverage, by the exact Euclidean distance transform of Felzenszwalb and Huttenlocher ("Distance Transforms of Sampled Functions", 2012), seeded at partly covered pixels with their distance to the edge, as Mapbox's TinySDF does.

/// Farther than any distance in a tile, but finite, so that the transform's arithmetic stays finite.
const FAR: f32 = 1e20;
/// Coverage within this of none or all is taken as none or all. A rasteriser's sums leave such traces in pixels wholly outside or inside an outline, as `ab_glyph` does in many of Noto Sans JP's glyphs, and each would otherwise count as lying on an edge, raising the field across the glyph's box to nearly a half, which drew a faint box around the glyph where a pixel spans more than a texel.
const TRACE: f32 = 1.0 / 256.0;

/// The field for `coverage`, `width` pixels wide and row by row from the top, each from 0 to 1: 0.5 on the edge, rising inside and falling outside by 0.5 over `spread` pixels.
pub(crate) fn field(coverage: &[f32], width: usize, spread: f32) -> Vec<u8> {
    let height = coverage.len() / width;
    // Squared distances to the nearest pixel outside, and inside.
    let mut outside = vec![FAR; coverage.len()];
    let mut inside = vec![0.0; coverage.len()];
    for (index, &alpha) in coverage.iter().enumerate() {
        if alpha >= 1.0 - TRACE {
            outside[index] = 0.0;
            inside[index] = FAR;
        } else if alpha > TRACE {
            let edge = 0.5 - alpha;
            outside[index] = if edge > 0.0 { edge * edge } else { 0.0 };
            inside[index] = if edge < 0.0 { edge * edge } else { 0.0 };
        }
    }
    transform(&mut outside, width, height);
    transform(&mut inside, width, height);
    outside
        .iter()
        .zip(&inside)
        .map(|(outside, inside)| {
            let distance = outside.sqrt() - inside.sqrt();
            ((0.5 - distance / (2.0 * spread)).clamp(0.0, 1.0) * 255.0).round() as u8
        })
        .collect()
}

/// The squared distance transform of `grid` in place, along columns then rows.
fn transform(grid: &mut [f32], width: usize, height: usize) {
    let longest = width.max(height);
    let mut line = vec![0.0; longest];
    let mut result = vec![0.0; longest];
    let mut parabolas = vec![0usize; longest];
    let mut bounds = vec![0.0; longest + 1];
    for x in 0..width {
        for y in 0..height {
            line[y] = grid[y * width + x];
        }
        lower_envelope(&line[..height], &mut result, &mut parabolas, &mut bounds);
        for y in 0..height {
            grid[y * width + x] = result[y];
        }
    }
    for y in 0..height {
        line[..width].copy_from_slice(&grid[y * width..(y + 1) * width]);
        lower_envelope(&line[..width], &mut result, &mut parabolas, &mut bounds);
        grid[y * width..(y + 1) * width].copy_from_slice(&result[..width]);
    }
}

/// The one-dimensional transform of `f` into `result`: for each position, the least squared distance to a position plus its value, from the lower envelope of the parabolas rooted at each.
fn lower_envelope(f: &[f32], result: &mut [f32], parabolas: &mut [usize], bounds: &mut [f32]) {
    let n = f.len();
    let mut k = 0;
    parabolas[0] = 0;
    bounds[0] = -FAR;
    bounds[1] = FAR;
    // Where the parabola rooted at `q` meets the one at `p`, from which on it is lower.
    let meets = |q: usize, p: usize| {
        let at = |p: usize| f[p] + (p * p) as f32;
        (at(q) - at(p)) / (2.0 * (q as f32 - p as f32))
    };
    for q in 1..n {
        // Parabolas the new one hides entirely are dropped. The first bound is below any meeting point, as every value is at most `FAR`, so the first parabola is never dropped this way.
        let mut s = meets(q, parabolas[k]);
        while s <= bounds[k] {
            k -= 1;
            s = meets(q, parabolas[k]);
        }
        k += 1;
        parabolas[k] = q;
        bounds[k] = s;
        bounds[k + 1] = FAR;
    }
    k = 0;
    for (q, value) in result.iter_mut().enumerate().take(n) {
        while bounds[k + 1] < q as f32 {
            k += 1;
        }
        let p = parabolas[k];
        let offset = q as f32 - p as f32;
        *value = offset * offset + f[p];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The field's value at `x`, `y`, from 0 to 1.
    fn at(field: &[u8], width: usize, x: usize, y: usize) -> f32 {
        f32::from(field[y * width + x]) / 255.0
    }

    #[test]
    fn a_square_is_half_on_its_edge_and_falls_away_outside() {
        // A 4 × 4 square, solid, in a 16 × 16 tile.
        let width = 16;
        let mut coverage = vec![0.0; width * width];
        for y in 6..10 {
            for x in 6..10 {
                coverage[y * width + x] = 1.0;
            }
        }
        let field = field(&coverage, width, 4.0);
        // Inside, above the edge; outside, below; far away, nothing.
        assert!(at(&field, width, 7, 7) > 0.5);
        assert!(at(&field, width, 4, 7) < 0.5);
        assert_eq!(at(&field, width, 0, 0), 0.0);
        // Distances are between pixels' centres, so the edge lies halfway between the last pixel inside and the first outside, and a pixel two from the square is two out.
        assert!((at(&field, width, 5, 7) + at(&field, width, 6, 7) - 1.0).abs() < 0.01);
        assert!((at(&field, width, 4, 7) - (0.5 - 2.0 / 8.0)).abs() < 0.01);
        // Distance is Euclidean: diagonally from the corner is farther than straight out.
        assert!(at(&field, width, 4, 4) < at(&field, width, 4, 7));
        // Symmetric left and right.
        assert_eq!(at(&field, width, 3, 7), at(&field, width, 12, 7));
    }

    #[test]
    fn partial_coverage_puts_the_edge_within_the_pixel() {
        let width = 8;
        let mut coverage = vec![0.0; width];
        coverage[..3].fill(1.0);
        coverage[3] = 0.5;
        let field = field(&coverage, width, 2.0);
        // A pixel half covered lies on the edge.
        assert!((at(&field, width, 3, 0) - 0.5).abs() < 0.01);
        let mut lighter = coverage.clone();
        lighter[3] = 0.25;
        let lighter = super::field(&lighter, width, 2.0);
        assert!(at(&lighter, width, 3, 0) < 0.5);
        let mut darker = coverage;
        darker[3] = 0.75;
        let darker = super::field(&darker, width, 2.0);
        assert!(at(&darker, width, 3, 0) > 0.5);
    }

    #[test]
    fn traces_of_coverage_are_none_or_all() {
        // A square as a rasteriser may leave it: traces of coverage outside it and a little short of all inside.
        let width = 16;
        let mut coverage = vec![1e-5; width * width];
        for y in 6..10 {
            for x in 6..10 {
                coverage[y * width + x] = 0.9999;
            }
        }
        let traced = field(&coverage, width, 4.0);
        let clean = field(
            &coverage
                .iter()
                .map(|alpha| alpha.round())
                .collect::<Vec<_>>(),
            width,
            4.0,
        );
        assert_eq!(traced, clean);
        assert_eq!(at(&traced, width, 0, 0), 0.0);
    }

    #[test]
    fn the_envelope_takes_the_nearest_root() {
        let f = [FAR, 0.0, FAR, FAR, 0.0, FAR, FAR];
        let mut result = [0.0; 7];
        let mut parabolas = [0; 7];
        let mut bounds = [0.0; 8];
        lower_envelope(&f, &mut result, &mut parabolas, &mut bounds);
        assert_eq!(result, [1.0, 0.0, 1.0, 1.0, 0.0, 1.0, 4.0]);
        // A low root hides higher ones entirely.
        let f = [3.0, 9.0, 0.0, 9.0];
        lower_envelope(&f, &mut result[..4], &mut parabolas, &mut bounds);
        assert_eq!(result[..4], [3.0, 1.0, 0.0, 1.0]);
    }
}
