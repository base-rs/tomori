// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Numbers that are the same everywhere, in every build, for what the farmyard's server and clients must agree on, as the
//! flock and the clucks, and for what is rendered alike each time, as its sounds. [`crate::props::scatter`] scatters
//! things alike.

/// The SplitMix64 finalizer of `value`.
pub fn mix(value: u64) -> u64 {
    let mut z = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// A fixed noise from `seed`, from −1 up to 1, the same every time: a linear congruential generator's top 24 bits.
pub fn noise(seed: u32) -> impl FnMut() -> f32 {
    let mut state = seed;
    move || {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (state >> 8) as f32 / (1u32 << 23) as f32 - 1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mix_is_the_splitmix64_finalizer() {
        // SplitMix64's first outputs from a state of 0, each the finalizer of the state advanced by its increment.
        assert_eq!(mix(0), 0xe220_a839_7b1d_cdaf);
        assert_eq!(mix(0x9e37_79b9_7f4a_7c15), 0x6e78_9e6a_a1b9_65f4);
    }

    #[test]
    fn noise_is_the_same_every_time_and_within_its_range() {
        let first: Vec<f32> = std::iter::repeat_with(noise(7)).take(1_000).collect();
        let again: Vec<f32> = std::iter::repeat_with(noise(7)).take(1_000).collect();
        assert_eq!(first, again);
        assert!(first.iter().all(|value| (-1.0..1.0).contains(value)));
        assert_ne!(
            first[..10],
            std::iter::repeat_with(noise(8))
                .take(10)
                .collect::<Vec<_>>()[..]
        );
        // The first from a seed of 0 is the generator's increment's top 24 bits.
        assert_eq!(
            noise(0)(),
            (1_013_904_223u32 >> 8) as f32 / (1u32 << 23) as f32 - 1.0
        );
    }
}
