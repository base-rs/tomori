// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Ducking: agent speech eases down while a person speaks, so that people talking with each other come first, and eases back once they stop; and an environment's ambience eases further down while anyone speaks, the agent included, so that it never competes with a conversation. Neither is ever stopped: each plays on underneath. See `design/spatial-audio.md`.
use std::time::Duration;

/// How far agent speech is lowered while a person speaks.
pub const DEPTH_DB: f32 = 8.0;
/// A person counts as speaking while their level, as [`super::Mixer::source_level`] measures it, is at least this: about −34 dBFS.
pub const THRESHOLD: f32 = 0.02;
/// A person must speak this long before agent speech ducks, so that a cough or a click does not.
pub const ONSET: Duration = Duration::from_millis(100);
/// Quiet this short, as between syllables, does not end the speaking that [`ONSET`] counts.
pub const GAP: Duration = Duration::from_millis(60);
/// Agent speech stays ducked through pauses this short, so that it does not pump between words.
pub const HOLD: Duration = Duration::from_millis(250);
/// How long agent speech takes to go down by [`DEPTH_DB`], and to come back up.
pub const ATTACK: Duration = Duration::from_millis(80);
pub const RELEASE: Duration = Duration::from_millis(500);

/// How a duck goes down and comes back up: how far, how long speaking must last first and through what gaps, how long it stays down through a pause, and how long it takes to go down and to come back.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shape {
    pub depth: f32,
    pub onset: Duration,
    pub gap: Duration,
    pub hold: Duration,
    pub attack: Duration,
    pub release: Duration,
}

impl Shape {
    /// Agent speech under a person speaking.
    pub const AGENT: Self = Self {
        depth: DEPTH_DB,
        onset: ONSET,
        gap: GAP,
        hold: HOLD,
        attack: ATTACK,
        release: RELEASE,
    };

    /// An environment's ambience under anyone speaking, the agent included: further down, and slower to come back, so
    /// that it does not swell in each pause between sentences.
    pub const AMBIENCE: Self = Self {
        depth: 12.0,
        onset: ONSET,
        gap: GAP,
        hold: Duration::from_millis(400),
        attack: Duration::from_millis(100),
        release: Duration::from_secs(1),
    };
}

/// The ducking envelope, advanced once per mix.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Duck {
    shape: Shape,
    /// How long, in samples, people have been speaking, or quiet.
    loud: u64,
    quiet: u64,
    engaged: bool,
    /// How far down it is now, in decibels, from 0 to its shape's depth.
    depth: f32,
}

impl Default for Duck {
    /// Agent speech's.
    fn default() -> Self {
        Self::new(Shape::AGENT)
    }
}

fn samples(duration: Duration, sample_rate: u32) -> u64 {
    (duration.as_secs_f64() * f64::from(sample_rate)).round() as u64
}

/// The gain for `depth` decibels down.
fn gain(depth: f32) -> f32 {
    10.0_f32.powf(-depth / 20.0)
}

impl Duck {
    pub fn new(shape: Shape) -> Self {
        Self {
            shape,
            loud: 0,
            quiet: 0,
            engaged: false,
            depth: 0.0,
        }
    }

    /// Advance by `frames` at `sample_rate`, with whether anyone it ducks under spoke above [`THRESHOLD`] when last measured. Returns the gain at the start and at the end of those frames, to glide between.
    pub fn update(&mut self, loud: bool, frames: usize, sample_rate: u32) -> (f32, f32) {
        let shape = self.shape;
        let frames = frames as u64;
        if loud {
            self.loud += frames;
            self.quiet = 0;
        } else {
            self.quiet += frames;
            if self.quiet >= samples(shape.gap, sample_rate) {
                self.loud = 0;
            }
        }
        if self.loud >= samples(shape.onset, sample_rate) {
            self.engaged = true;
        } else if self.quiet >= samples(shape.hold, sample_rate) {
            self.engaged = false;
        }
        let start = self.depth;
        let (target, over) = if self.engaged {
            (shape.depth, shape.attack)
        } else {
            (0.0, shape.release)
        };
        let step = shape.depth * frames as f32 / samples(over, sample_rate).max(1) as f32;
        self.depth = if target > self.depth {
            (self.depth + step).min(target)
        } else {
            (self.depth - step).max(target)
        };
        (gain(start), gain(self.depth))
    }

    /// How far down it is now, in decibels.
    pub fn depth(&self) -> f32 {
        self.depth
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 1_000;

    #[test]
    fn ducks_only_after_a_person_speaks_for_the_onset() {
        let mut duck = Duck::default();
        // 90 ms of speech: not yet.
        for _ in 0..9 {
            assert_eq!(duck.update(true, 10, RATE), (1.0, 1.0));
        }
        let (start, end) = duck.update(true, 10, RATE);
        assert_eq!(start, 1.0);
        assert!(end < 1.0);
        // A click shorter than the onset, after quiet, never ducks.
        let mut click = Duck::default();
        click.update(true, 50, RATE);
        click.update(false, 300, RATE);
        click.update(true, 50, RATE);
        assert_eq!(click.depth(), 0.0);
        // Syllables with short gaps between them add up to speaking.
        let mut syllables = Duck::default();
        for _ in 0..3 {
            syllables.update(true, 40, RATE);
            syllables.update(false, 30, RATE);
        }
        assert!(syllables.depth() > 0.0);
    }

    #[test]
    fn goes_down_over_the_attack_and_back_over_the_release_after_the_hold() {
        let mut duck = Duck::default();
        duck.update(true, 90, RATE);
        // 80 ms down by 8 dB, in steps of 10 ms: 1 dB each.
        let mut depths = Vec::new();
        for _ in 0..8 {
            duck.update(true, 10, RATE);
            depths.push(duck.depth());
        }
        assert!((depths[0] - 1.0).abs() < 1e-4, "{depths:?}");
        assert!(depths.windows(2).all(|pair| pair[1] >= pair[0]));
        assert_eq!(duck.depth(), DEPTH_DB);
        let (_, low) = duck.update(true, 10, RATE);
        assert!((low - gain(8.0)).abs() < 1e-6 && (low - 0.398).abs() < 1e-3);
        // A pause between words, shorter than the hold, keeps it down.
        duck.update(false, 240, RATE);
        assert_eq!(duck.depth(), DEPTH_DB);
        // Then it comes back up over half a second.
        duck.update(false, 10, RATE);
        let mut rising = 0;
        while duck.depth() > 0.0 {
            let (start, end) = duck.update(false, 10, RATE);
            assert!(end > start);
            rising += 10;
        }
        assert!((480..=500).contains(&rising), "{rising}");
        assert_eq!(duck.update(false, 10, RATE), (1.0, 1.0));
    }

    #[test]
    fn ambience_goes_down_by_12_db_over_100_ms_and_back_over_a_second_after_a_longer_hold() {
        let mut duck = Duck::new(Shape::AMBIENCE);
        // Speaking for the onset engages it, as for agent speech.
        duck.update(true, 90, RATE);
        assert_eq!(duck.depth(), 0.0);
        let mut attack = 0;
        while duck.depth() < 12.0 {
            duck.update(true, 10, RATE);
            attack += 10;
        }
        assert!((100..=110).contains(&attack), "{attack}");
        let (_, low) = duck.update(true, 10, RATE);
        assert!((low - gain(12.0)).abs() < 1e-6 && (low - 0.251).abs() < 1e-3);
        // A pause between sentences, shorter than its hold, keeps it down, where agent speech would come back.
        duck.update(false, 390, RATE);
        assert_eq!(duck.depth(), 12.0);
        duck.update(false, 10, RATE);
        let mut release = 0;
        while duck.depth() > 0.0 {
            let (start, end) = duck.update(false, 10, RATE);
            assert!(end > start);
            release += 10;
        }
        assert!((980..=1_000).contains(&release), "{release}");
        assert_eq!(Duck::default(), Duck::new(Shape::AGENT));
    }
}
