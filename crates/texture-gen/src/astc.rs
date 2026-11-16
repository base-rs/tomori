// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! ASTC's 4×4 blocks, as the Quest's GPU samples them, 8 bits a texel, in one configuration only: one partition, one
//! plane, a 4×4 grid of weights of 8 levels each, 3 bits, and its two RGB endpoints direct (colour endpoint mode 8) of
//! 256 levels, 8 bits each, so that every value is whole bits and none needs ASTC's trits or quints. Each block's
//! endpoints are fitted along its colours' principal axis, then refined by least squares, its weights chosen exactly
//! against how a GPU decodes them. See ARM's ASTC specification, in the Khronos Data Format Specification, chapter 23.
use crate::fit;

/// The block mode: a 4×4 grid of weights, one plane, weights of 8 levels (`QUANT_8`).
const BLOCK_MODE: u128 = 0x53;
/// Colour endpoint mode 8, LDR RGB direct.
const MODE: u128 = 8;
/// What each of the 8 weight levels is, out of 64, as a decoder unquantizes them: 3 bits replicated to 6, and one more
/// above 32.
const WEIGHTS: [u32; 8] = [0, 9, 18, 27, 37, 46, 55, 64];

/// What a texel of a block with endpoints `low` and `high` decodes to at weight `weight`, in bytes' units, as a GPU
/// reads it: in sRGB, its top 8 bits, which are then decoded from sRGB; in linear, its 16 bits, as a fraction.
fn decoded(low: u8, high: u8, weight: u32, srgb: bool) -> f32 {
    let expand = |value: u8| {
        if srgb {
            u32::from(value) << 8 | 0x80
        } else {
            u32::from(value) * 257
        }
    };
    let value = (expand(low) * (64 - weight) + expand(high) * weight + 32) >> 6;
    if srgb {
        (value >> 8) as f32
    } else {
        value as f32 / 257.0
    }
}

/// The squared error of `texels`, RGB, with endpoints `ends`, each texel at its best weight, and those weights.
fn weigh(texels: &[[u8; 4]; 16], ends: [[u8; 3]; 2], srgb: bool) -> (f32, [u8; 16]) {
    let mut chosen = [0u8; 16];
    let mut total = 0.0;
    for (texel, chosen) in texels.iter().zip(&mut chosen) {
        let mut best = f32::MAX;
        for (level, weight) in WEIGHTS.iter().enumerate() {
            let error: f32 = (0..3)
                .map(|channel| {
                    let difference = decoded(ends[0][channel], ends[1][channel], *weight, srgb)
                        - f32::from(texel[channel]);
                    difference * difference
                })
                .sum();
            if error < best {
                best = error;
                *chosen = level as u8;
            }
        }
        total += best;
    }
    (total, chosen)
}

/// One 4×4 block of RGB `texels`, each in its row's order, the rows top first, their alpha ignored and drawn opaque, as
/// 16 bytes of ASTC, its colours encoded as sRGB if `srgb`, otherwise linear.
pub fn block(texels: &[[u8; 4]; 16], srgb: bool) -> [u8; 16] {
    let fractions = WEIGHTS.map(|weight| weight as f32 / 64.0);
    let mut best = {
        let ends = fit::principal(texels);
        let (error, weights) = weigh(texels, ends, srgb);
        (error, ends, weights)
    };
    // Least squares, given the weights chosen, twice, keeping what is better.
    for _ in 0..2 {
        let at = best.2.map(|level| fractions[usize::from(level)]);
        let Some(ends) = fit::least_squares(texels, &at) else {
            break;
        };
        let (error, weights) = weigh(texels, ends, srgb);
        if error >= best.0 {
            break;
        }
        best = (error, ends, weights);
    }
    let (_, mut ends, mut weights) = best;
    // A decoder swaps the endpoints, and contracts blue, if the first's sum is the greater: order them so it is not,
    // turning the weights about, which the levels are symmetric for.
    let sum = |end: [u8; 3]| end.iter().map(|value| u32::from(*value)).sum::<u32>();
    if sum(ends[1]) < sum(ends[0]) {
        ends.swap(0, 1);
        weights = weights.map(|level| 7 - level);
    }
    let mut bits = BLOCK_MODE | MODE << 13;
    // The endpoints from bit 17 up: red's low and high, then green's, then blue's.
    for channel in 0..3 {
        for (end, values) in ends.iter().enumerate() {
            bits |= u128::from(values[channel]) << (17 + 8 * (channel * 2 + end));
        }
    }
    // The weights from the top bit down, their bits in reverse.
    for (index, level) in weights.iter().enumerate() {
        for bit in 0..3 {
            if level >> bit & 1 == 1 {
                bits |= 1 << (127 - (3 * index + bit));
            }
        }
    }
    bits.to_le_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(block: [u8; 16], srgb: bool) -> [[u8; 4]; 16] {
        let mut texels = [[0u8; 4]; 16];
        assert!(astc_decode::astc_decode_block(
            &block,
            astc_decode::Footprint::new(4, 4),
            |x, y, color| {
                texels[(y * 4 + x) as usize] = color;
            }
        ));
        let _ = srgb;
        texels
    }

    fn block_of(texel: impl Fn(usize) -> [u8; 4]) -> [[u8; 4]; 16] {
        std::array::from_fn(texel)
    }

    #[test]
    fn a_block_is_read_back_by_another_decoder_as_it_was_meant() {
        // A ramp across, a ramp of two colours, a flat colour, noise, and colours whose sums would have the decoder
        // swap them.
        let cases = [
            block_of(|index| [(index % 4 * 60) as u8, 40, 200, 255]),
            block_of(|index| {
                let t = index as u32 * 17;
                [(255 - t) as u8, (t / 2) as u8, 30, 255]
            }),
            block_of(|_| [90, 120, 33, 255]),
            block_of(|index| {
                let hash = crate::noise::hash(index as u32, 0, 4);
                [hash as u8, (hash >> 8) as u8, (hash >> 16) as u8, 255]
            }),
            block_of(|index| {
                if index < 8 {
                    [250, 250, 250, 255]
                } else {
                    [3, 2, 1, 255]
                }
            }),
        ];
        for (case, texels) in cases.iter().enumerate() {
            for srgb in [false, true] {
                let encoded = block(texels, srgb);
                let read = decode(encoded, srgb);
                for (index, (got, want)) in read.iter().zip(texels).enumerate() {
                    assert_eq!(got[3], 255, "{case} {index}");
                    for channel in 0..3 {
                        let error = (i32::from(got[channel]) - i32::from(want[channel])).abs();
                        // Exactly as this encoder expects it decoded, which in sRGB is its top byte.
                        let (ends, levels) = {
                            let ends = [
                                [0, 2, 4].map(|at| {
                                    ((u128::from_le_bytes(encoded) >> (17 + 8 * at)) & 0xff) as u8
                                }),
                                [1, 3, 5].map(|at| {
                                    ((u128::from_le_bytes(encoded) >> (17 + 8 * at)) & 0xff) as u8
                                }),
                            ];
                            let bits = u128::from_le_bytes(encoded);
                            let level = (0..3).fold(0u32, |level, bit| {
                                level | ((bits >> (127 - (3 * index + bit)) & 1) as u32) << bit
                            });
                            (ends, level)
                        };
                        let meant = decoded(
                            ends[0][channel],
                            ends[1][channel],
                            WEIGHTS[levels as usize],
                            srgb,
                        );
                        assert!(
                            (f32::from(got[channel]) - meant).abs() <= 0.51,
                            "{case} {index} {got:?} {meant}"
                        );
                        // A flat colour and two colours closely, ramps within half of one of the 8 weights' steps,
                        // and noise, which no line through colours holds, loosely.
                        let bound = match case {
                            2 | 4 => 4,
                            3 => 160,
                            _ => 20,
                        };
                        assert!(
                            error <= bound,
                            "{case} {srgb} {index}: {got:?} for {want:?}"
                        );
                    }
                }
            }
        }
    }
}
