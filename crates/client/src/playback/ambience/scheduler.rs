// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! When each of an ambience's calls is heard next, which of its variants, from where, at what pitch and how loud:
//! drawn from a seeded random sequence, so that the same seed always gives the same calls, while each client seeds its
//! own and hears its own. See `design/spatial-audio.md`.
use std::time::Duration;

use super::{Call, Order, RUN_CALLS};
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

/// A run being heard: how many of its calls are left, and where its bird is, and its first call's pitch and gain, in
/// decibels, about which the others vary.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Running {
    left: usize,
    position: Position,
    pitch: f32,
    gain: f32,
}

/// When each call is next heard, and the random sequence that decides it.
#[derive(Clone, Debug)]
pub struct Scheduler {
    next: Vec<Duration>,
    /// The variant each call was last heard as, so that one with several is not heard as the same twice running.
    last: Vec<Option<usize>>,
    /// The run each call is in, if any.
    running: Vec<Option<Running>>,
    /// How long each call's variants last, at their own pitch.
    lengths: Vec<Vec<Duration>>,
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
            running: vec![None; calls.len()],
            lengths: Vec::new(),
            random,
        }
    }

    /// This schedule knowing how long each call's variants last, at their own pitch, from which a run's gaps and the
    /// interval after it count; one not given lasts no time.
    pub fn with_lengths(self, lengths: Vec<Vec<Duration>>) -> Self {
        Self { lengths, ..self }
    }

    /// When each call is next heard.
    pub fn next(&self) -> &[Duration] {
        &self.next
    }

    /// The call due soonest, if any is due by `now`, which is then heard again after an interval within its `every`,
    /// and at least [`LEAST`], from `now`, so that calls held back, as while ambience is off, do not come in a burst.
    /// A call with a run is heard again within its gap of its end until the run is over, from the same place, then
    /// after an interval within its `every` of the run's end; a run held back past its longest gap begins again.
    pub fn due(&mut self, calls: &[Call], now: Duration) -> Option<Due> {
        let (index, at) = self
            .next
            .iter()
            .copied()
            .enumerate()
            .filter(|(_, at)| *at <= now)
            .min_by_key(|(_, at)| *at)?;
        let call = &calls[index];
        let Some(run) = &call.run else {
            let interval = self.interval(call.every);
            self.next[index] = now + interval.max(LEAST);
            let (variant, position, pitch, gain) = self.draw(index, call);
            return Some(Due {
                call: index,
                variant,
                position,
                pitch,
                gain: super::gain(gain),
            });
        };
        let (variant, pitch, gain, running) = match self.running[index].take() {
            Some(running) if now <= at + run.gap[1] => {
                let count = call.variants.len().max(1);
                let variant = match (run.order, self.last[index]) {
                    (Order::Alternate, Some(last)) => (last + 1) % count,
                    _ => self.random.below(count),
                };
                self.last[index] = Some(variant);
                let [pitch, gain] = run.vary;
                // Within the call's own ranges, as `max` and `min` keep it however they are given.
                let pitch = (running.pitch * self.random.between(1.0 - pitch, 1.0 + pitch))
                    .max(call.pitch[0])
                    .min(call.pitch[1]);
                let gain = (running.gain + self.random.between(-gain, gain))
                    .max(call.gain[0])
                    .min(call.gain[1]);
                let running = Running {
                    left: running.left - 1,
                    ..running
                };
                (variant, pitch, gain, running)
            }
            _ => {
                let (variant, position, pitch, gain) = self.draw(index, call);
                let running = Running {
                    left: self.length(&run.weights) - 1,
                    position,
                    pitch,
                    gain,
                };
                (variant, pitch, gain, running)
            }
        };
        let position = running.position;
        let length = self
            .lengths
            .get(index)
            .and_then(|lengths| lengths.get(variant))
            .copied()
            .unwrap_or_default();
        let end = now + length.div_f64(f64::from(pitch.max(0.01)));
        let after = if running.left > 0 {
            self.running[index] = Some(running);
            self.interval(run.gap)
        } else {
            self.interval(call.every)
        };
        self.next[index] = (end + after).max(now + LEAST);
        Some(Due {
            call: index,
            variant,
            position,
            pitch,
            gain: super::gain(gain),
        })
    }

    /// A time within `range`.
    fn interval(&mut self, range: [Duration; 2]) -> Duration {
        range[0] + (range[1] - range[0]).mul_f64(self.random.unit())
    }

    /// How many calls a run has, by `weights`, up to [`RUN_CALLS`], those not finite or below 0 never: one, for none.
    fn length(&mut self, weights: &[f32]) -> usize {
        let weight = |weight: &f32| {
            if weight.is_finite() {
                weight.max(0.0)
            } else {
                0.0
            }
        };
        let weights = &weights[..weights.len().min(RUN_CALLS)];
        let total: f32 = weights.iter().map(weight).sum();
        let mut pick = self.random.between(0.0, total);
        let mut length = 1;
        for (index, weight) in weights.iter().map(weight).enumerate() {
            if weight > 0.0 {
                length = index + 1;
                if pick < weight {
                    break;
                }
                pick -= weight;
            }
        }
        length
    }

    /// Call `index`'s variant, never the one it was last heard as, its place, its pitch and its gain, in decibels.
    fn draw(&mut self, index: usize, call: &Call) -> (usize, Position, f32, f32) {
        let random = &mut self.random;
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
        let gain = random.between(call.gain[0], call.gain[1]);
        (variant, position, pitch, gain)
    }
}
