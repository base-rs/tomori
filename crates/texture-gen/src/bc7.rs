// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! BC7's 4×4 blocks, as desktop GPUs sample them, 8 bits a texel, in mode 6 alone: one subset, RGBA endpoints of 7 bits
//! and a shared bit each, and 16 weights of 4 bits. Each block's endpoints are fitted along its colours' principal axis,
//! then by least squares, and each endpoint's shared bit and each texel's weight chosen exactly against how a GPU
//! decodes them. See the BC7 format in the Khronos Data Format Specification, chapter 21.
use crate::fit;

/// What each of the 16 weights is, out of 64.
const WEIGHTS: [u32; 16] = [0, 4, 9, 13, 17, 21, 26, 30, 34, 38, 43, 47, 51, 55, 60, 64];

/// An endpoint as stored: 7 bits a channel, RGBA, and its shared lowest bit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct End {
    channels: [u8; 4],
    bit: u8,
}

impl End {
    /// The endpoint nearest `color`, RGB, opaque, with shared bit `bit`.
    fn nearest(color: [u8; 3], bit: u8) -> Self {
        let quantize =
            |value: u8| ((i32::from(value) - i32::from(bit) + 1) / 2).clamp(0, 127) as u8;
        Self {
            channels: [
                quantize(color[0]),
                quantize(color[1]),
                quantize(color[2]),
                127,
            ],
            bit,
        }
    }

    /// Its channels, 8 bits each, as a decoder reads them.
    fn color(self) -> [u32; 4] {
        self.channels
            .map(|value| u32::from(value) << 1 | u32::from(self.bit))
    }
}

/// The squared error of `texels`, RGBA, with endpoints `ends`, each texel at its best weight, and those weights.
fn weigh(texels: &[[u8; 4]; 16], ends: [End; 2]) -> (u32, [u8; 16]) {
    let [low, high] = ends.map(End::color);
    let mut chosen = [0u8; 16];
    let mut total = 0;
    for (texel, chosen) in texels.iter().zip(&mut chosen) {
        let mut best = u32::MAX;
        for (index, weight) in WEIGHTS.iter().enumerate() {
            let error: u32 = (0..4)
                .map(|channel| {
                    let value = (low[channel] * (64 - weight) + high[channel] * weight + 32) >> 6;
                    let difference = value.abs_diff(u32::from(texel[channel]));
                    difference * difference
                })
                .sum();
            if error < best {
                best = error;
                *chosen = index as u8;
            }
        }
        total += best;
    }
    (total, chosen)
}

/// The best of the four choices of shared bits for endpoints `colors`.
fn best(texels: &[[u8; 4]; 16], colors: [[u8; 3]; 2]) -> (u32, [End; 2], [u8; 16]) {
    let mut best = (u32::MAX, [End::nearest(colors[0], 0); 2], [0; 16]);
    for bits in 0..4u8 {
        let ends = [
            End::nearest(colors[0], bits & 1),
            End::nearest(colors[1], bits >> 1),
        ];
        let (error, weights) = weigh(texels, ends);
        if error < best.0 {
            best = (error, ends, weights);
        }
    }
    best
}

/// One 4×4 block of `texels`, each in its row's order, the rows top first, drawn opaque, as 16 bytes of BC7.
pub fn block(texels: &[[u8; 4]; 16]) -> [u8; 16] {
    let fractions = WEIGHTS.map(|weight| weight as f32 / 64.0);
    let mut chosen = best(texels, fit::principal(texels));
    for _ in 0..2 {
        let at = chosen.2.map(|index| fractions[usize::from(index)]);
        let Some(colors) = fit::least_squares(texels, &at) else {
            break;
        };
        let next = best(texels, colors);
        if next.0 >= chosen.0 {
            break;
        }
        chosen = next;
    }
    let (_, mut ends, mut weights) = chosen;
    // The first texel's weight is stored without its top bit, so it must be less than 8: swap the endpoints and turn the
    // weights about if not.
    if weights[0] >= 8 {
        ends.swap(0, 1);
        weights = weights.map(|index| 15 - index);
    }
    // Mode 6: six zeros and a one, the endpoints by channel, the low then the high, the shared bits, then the weights.
    let mut bits: u128 = 1 << 6;
    let mut at = 7;
    let mut put = |value: u128, width: u32| {
        bits |= value << at;
        at += width;
    };
    for channel in 0..4 {
        for end in &ends {
            put(u128::from(end.channels[channel]), 7);
        }
    }
    put(u128::from(ends[0].bit), 1);
    put(u128::from(ends[1].bit), 1);
    for (index, weight) in weights.iter().enumerate() {
        put(u128::from(*weight), if index == 0 { 3 } else { 4 });
    }
    bits.to_le_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_block_is_read_back_by_another_decoder_closely() {
        let cases: [[[u8; 4]; 16]; 4] = [
            std::array::from_fn(|index| [(index % 4 * 60) as u8, 40, 200, 255]),
            std::array::from_fn(|index| [(255 - index * 17) as u8, (index * 8) as u8, 30, 255]),
            [[90, 120, 33, 255]; 16],
            std::array::from_fn(|index| {
                let hash = crate::noise::hash(index as u32, 0, 4);
                [hash as u8, (hash >> 8) as u8, (hash >> 16) as u8, 255]
            }),
        ];
        for (case, texels) in cases.iter().enumerate() {
            let block = block(texels);
            let mut decoded = [0u8; 64];
            bcdec_rs::bc7(&block, &mut decoded, 16);
            for (index, want) in texels.iter().enumerate() {
                let got = &decoded[index * 4..index * 4 + 4];
                assert!(got[3] >= 254, "{case} {index}");
                for channel in 0..3 {
                    let error = (i32::from(got[channel]) - i32::from(want[channel])).abs();
                    // Ramps within half of one of the 16 weights' steps, and noise, which no line through colours
                    // holds, loosely.
                    let bound = if case == 3 { 160 } else { 10 };
                    assert!(error <= bound, "{case} {index}: {got:?} for {want:?}");
                }
            }
        }
        // The first weight's top bit is never set.
        let swapped: [[u8; 4]; 16] = std::array::from_fn(|index| {
            let value = 255 - index as u8 * 16;
            [value, value, value, 255]
        });
        let block = block(&swapped);
        assert_eq!(u128::from_le_bytes(block) & 0x7f, 1 << 6);
        let mut decoded = [0u8; 64];
        bcdec_rs::bc7(&block, &mut decoded, 16);
        let (first, last) = (decoded[0], decoded[60]);
        assert!(first >= 250 && last <= 20, "{first} {last}");
    }
}
