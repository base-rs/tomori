// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Fitting a block's two endpoints to its colours, as both encoders do: along their principal axis, then by least
//! squares for the weights chosen. Arithmetic alone, in a fixed order, so that it is the same on every machine.

/// The endpoints spanning `texels`' RGB along the axis they vary most along, found by power iteration.
pub fn principal(texels: &[[u8; 4]; 16]) -> [[u8; 3]; 2] {
    let colors = texels.map(|texel| [0, 1, 2].map(|channel| f32::from(texel[channel])));
    let mean: [f32; 3] = std::array::from_fn(|channel| {
        colors.iter().map(|color| color[channel]).sum::<f32>() / 16.0
    });
    let mut covariance = [[0.0f32; 3]; 3];
    for color in &colors {
        let offset: [f32; 3] = std::array::from_fn(|channel| color[channel] - mean[channel]);
        for row in 0..3 {
            for column in 0..3 {
                covariance[row][column] += offset[row] * offset[column];
            }
        }
    }
    // From how the channel that varies most varies with each, which, unlike the bounds' diagonal, is never across the
    // axis when channels vary against each other.
    let most = (0..3).fold(0, |most, channel| {
        if covariance[channel][channel] > covariance[most][most] {
            channel
        } else {
            most
        }
    });
    let mut axis = covariance[most];
    for _ in 0..8 {
        let next: [f32; 3] = std::array::from_fn(|row| {
            (0..3)
                .map(|column| covariance[row][column] * axis[column])
                .sum()
        });
        let length = next.iter().map(|value| value * value).sum::<f32>().sqrt();
        if length <= 1e-6 {
            break;
        }
        axis = next.map(|value| value / length);
    }
    let length = axis.iter().map(|value| value * value).sum::<f32>().sqrt();
    if length <= 1e-6 {
        // One colour.
        let color = mean.map(|value| value.round().clamp(0.0, 255.0) as u8);
        return [color, color];
    }
    let axis = axis.map(|value| value / length);
    let along = |color: &[f32; 3]| {
        (0..3)
            .map(|channel| (color[channel] - mean[channel]) * axis[channel])
            .sum::<f32>()
    };
    let (low, high) = colors
        .iter()
        .fold((f32::MAX, f32::MIN), |(low, high), color| {
            let at = along(color);
            (low.min(at), high.max(at))
        });
    let end = |at: f32| {
        std::array::from_fn(|channel| {
            (mean[channel] + axis[channel] * at)
                .round()
                .clamp(0.0, 255.0) as u8
        })
    };
    [end(low), end(high)]
}

/// The endpoints that best give `texels`' RGB, each texel at `at` of the way from the first to the second, by least
/// squares; or none if the texels are all at one place between them.
pub fn least_squares(texels: &[[u8; 4]; 16], at: &[f32; 16]) -> Option<[[u8; 3]; 2]> {
    let (mut aa, mut ab, mut bb) = (0.0f32, 0.0f32, 0.0f32);
    let (mut ac, mut bc) = ([0.0f32; 3], [0.0f32; 3]);
    for (texel, t) in texels.iter().zip(at) {
        let (a, b) = (1.0 - t, *t);
        aa += a * a;
        ab += a * b;
        bb += b * b;
        for channel in 0..3 {
            ac[channel] += a * f32::from(texel[channel]);
            bc[channel] += b * f32::from(texel[channel]);
        }
    }
    let determinant = aa * bb - ab * ab;
    if determinant.abs() < 1e-4 {
        return None;
    }
    let solve = |first: bool| -> [u8; 3] {
        std::array::from_fn(|channel| {
            let value = if first {
                (bb * ac[channel] - ab * bc[channel]) / determinant
            } else {
                (aa * bc[channel] - ab * ac[channel]) / determinant
            };
            value.round().clamp(0.0, 255.0) as u8
        })
    };
    Some([solve(true), solve(false)])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoints_span_a_ramp_and_least_squares_finds_them_again() {
        let ramp: [[u8; 4]; 16] = std::array::from_fn(|index| {
            let t = index as u8 * 10;
            [20 + t, 200 - t, 50, 255]
        });
        assert_eq!(principal(&ramp), [[20, 200, 50], [170, 50, 50]]);
        let at: [f32; 16] = std::array::from_fn(|index| index as f32 / 15.0);
        assert_eq!(
            least_squares(&ramp, &at),
            Some([[20, 200, 50], [170, 50, 50]])
        );
        // Every texel at one place says nothing of the endpoints.
        assert_eq!(least_squares(&ramp, &[0.5; 16]), None);
        // One colour is both endpoints.
        assert_eq!(principal(&[[7, 8, 9, 255]; 16]), [[7, 8, 9]; 2]);
    }
}
