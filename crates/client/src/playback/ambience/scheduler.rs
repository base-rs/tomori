// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! When each of an ambience's calls is heard next, which of its variants, from where, at what pitch and how loud:
//! drawn from a seeded random sequence, so that the same seed always gives the same calls, while each client seeds its
//! own and hears its own. See `design/spatial-audio.md`.
use std::time::Duration;

use super::Call;
use crate::playback::spatial::Position;

/// The shortest interval between two of a call, however short its own: at most ten a second.
pub const LEAST: Duration = Duration::from_millis(100);

/// A small, fast random sequence, SplitMix64, the same for the same seed on every platform.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Random(u64);

impl Random {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// A number from 0 up to, but not including, 1.
    pub fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// A number from `low` to `high`.
    pub fn between(&mut self, low: f32, high: f32) -> f32 {
        low + (high - low) * self.unit() as f32
    }

    /// A whole number below `count`, which must be at least 1.
    pub fn below(&mut self, count: usize) -> usize {
        ((self.unit() * count as f64) as usize).min(count - 1)
    }
}

/// A call that is due: which, which of its variants, from where, at what pitch, as a rate of playback, and at what
/// gain, as a factor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Due {
    pub call: usize,
    pub variant: usize,
    pub position: Position,
    pub pitch: f32,
    pub gain: f32,
}

/// When each call is next heard, and the random sequence that decides it.
#[derive(Clone, Debug)]
pub struct Scheduler {
    next: Vec<Duration>,
    /// The variant each call was last heard as, so that one with several is not heard as the same twice running.
    last: Vec<Option<usize>>,
    random: Random,
}

impl Scheduler {
    /// The calls' schedule from `now`, drawn from `seed`: each is first heard within its longest interval.
    pub fn new(calls: &[Call], seed: u64, now: Duration) -> Self {
        let mut random = Random::new(seed);
        let next = calls
            .iter()
            .map(|call| now + call.every[1].mul_f64(random.unit()))
            .collect();
        Self {
            next,
            last: vec![None; calls.len()],
            random,
        }
    }

    /// When each call is next heard.
    pub fn next(&self) -> &[Duration] {
        &self.next
    }

    /// The call due soonest, if any is due by `now`, which is then heard again after an interval within its `every`,
    /// and at least [`LEAST`], from `now`, so that calls held back, as while ambience is off, do not come in a burst.
    pub fn due(&mut self, calls: &[Call], now: Duration) -> Option<Due> {
        let (index, _) = self
            .next
            .iter()
            .enumerate()
            .filter(|(_, at)| **at <= now)
            .min_by_key(|(_, at)| **at)?;
        let call = &calls[index];
        let random = &mut self.random;
        let interval = call.every[0] + (call.every[1] - call.every[0]).mul_f64(random.unit());
        self.next[index] = now + interval.max(LEAST);
        let count = call.variants.len().max(1);
        let variant = match self.last[index] {
            // One of the others, so that the same is never heard twice running.
            Some(last) if count > 1 => (last + 1 + random.below(count - 1)) % count,
            _ => random.below(count),
        };
        self.last[index] = Some(variant);
        // A call declared around nowhere is heard from the origin.
        let point = call
            .around
            .get(random.below(call.around.len().max(1)))
            .copied()
            .unwrap_or_default();
        let angle = std::f64::consts::TAU * random.unit();
        let reach = f64::from(call.spread) * random.unit().sqrt();
        let position = [
            point[0] + (reach * angle.cos()) as f32,
            point[1],
            point[2] + (reach * angle.sin()) as f32,
        ];
        let pitch = random.between(call.pitch[0], call.pitch[1]);
        let gain = super::gain(random.between(call.gain[0], call.gain[1]));
        Some(Due {
            call: index,
            variant,
            position,
            pitch,
            gain,
        })
    }
}
