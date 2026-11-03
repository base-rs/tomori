// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Short sound cues, synthesized rather than loaded from files: each is a few sine partials per note with a quick attack and an exponential decay, described in [`Earcon::spec`]. Every cue is rendered once at the device rate when a [`super::Mixer`] is created, so that playing one only copies samples. See `design/feedback.md`.

use std::f64::consts::TAU;

/// A sound cue. Each marks a moment something changed; see `design/feedback.md` for when each plays.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Earcon {
    /// A soft click: something was chosen.
    Choose,
    /// A short rising tick: the microphone opened, for push-to-talk or live audio. Played through push-to-talk's silencing, since it plays as X is pressed.
    On,
    /// A short falling tick: the live microphone closed.
    Off,
    /// Two rising notes: the words were queued or answered.
    Sent,
    /// Two falling low notes: something failed, such as words that were not sent.
    Failed,
    /// A three-note chime: an approval or question waits.
    Request,
    /// The chime, quieter: the same request still waits.
    Reminder,
    /// A soft falling swell: speech played again to catch up has started.
    CatchingUp,
    /// Two soft knocks: someone knocks to take part.
    Knock,
    /// A low click: holding B left the space.
    Leave,
}

impl Earcon {
    /// Every cue, in the order a [`Bank`] holds them.
    pub const ALL: [Self; 10] = [
        Self::Choose,
        Self::On,
        Self::Off,
        Self::Sent,
        Self::Failed,
        Self::Request,
        Self::Reminder,
        Self::CatchingUp,
        Self::Knock,
        Self::Leave,
    ];

    fn index(self) -> usize {
        Self::ALL
            .iter()
            .position(|earcon| *earcon == self)
            .expect("every earcon is listed")
    }

    /// Whether the cue plays while push-to-talk silences playback: only the tick as the microphone opens, which plays as X is pressed. Anything else would be heard by the microphone and transcribed.
    pub fn bypasses_silence(self) -> bool {
        self == Self::On
    }

    /// How the cue is synthesized.
    pub fn spec(self) -> Spec {
        // A pentatonic family, so that cues sound related and never clash.
        const G4: f32 = 392.0;
        const C5: f32 = 523.25;
        const E5: f32 = 659.25;
        const G5: f32 = 783.99;
        const C6: f32 = 1046.5;
        const D6: f32 = 1174.66;
        const E6: f32 = 1318.51;
        const G6: f32 = 1567.98;
        const A6: f32 = 1760.0;
        const PURE: &[(f32, f32)] = &[(1.0, 1.0)];
        const BELL: &[(f32, f32)] = &[(1.0, 1.0), (2.0, 0.3), (3.0, 0.1)];
        const WOOD: &[(f32, f32)] = &[(1.0, 1.0), (1.5, 0.4), (2.5, 0.15)];
        let note = |start: u32, length: u32, from: f32, to: f32| Note {
            start,
            length,
            from,
            to,
        };
        match self {
            Self::Choose => Spec {
                notes: vec![note(0, 25, E6, E6)],
                partials: BELL,
                attack: 1,
                level: 0.10,
            },
            Self::On => Spec {
                notes: vec![note(0, 20, G5, G5), note(18, 22, C6, C6)],
                partials: PURE,
                attack: 2,
                level: 0.12,
            },
            Self::Off => Spec {
                notes: vec![note(0, 20, C6, C6), note(18, 22, G5, G5)],
                partials: PURE,
                attack: 2,
                level: 0.10,
            },
            Self::Sent => Spec {
                notes: vec![note(0, 70, C6, C6), note(80, 110, G6, G6)],
                partials: BELL,
                attack: 3,
                level: 0.10,
            },
            Self::Failed => Spec {
                notes: vec![note(0, 90, E5, E5), note(110, 140, C5, C5)],
                partials: WOOD,
                attack: 3,
                level: 0.14,
            },
            Self::Request => Spec {
                notes: vec![
                    note(0, 90, C6, C6),
                    note(80, 90, E6, E6),
                    note(160, 90, G6, G6),
                ],
                partials: BELL,
                attack: 3,
                level: 0.14,
            },
            Self::Reminder => Spec {
                level: 0.08,
                ..Self::Request.spec()
            },
            Self::CatchingUp => Spec {
                notes: vec![note(0, 220, A6, D6)],
                partials: PURE,
                attack: 80,
                level: 0.07,
            },
            Self::Knock => Spec {
                notes: vec![note(0, 35, G4, G4), note(120, 35, G4, G4)],
                partials: WOOD,
                attack: 1,
                level: 0.14,
            },
            Self::Leave => Spec {
                notes: vec![note(0, 30, C5, C5)],
                partials: WOOD,
                attack: 1,
                level: 0.12,
            },
        }
    }
}

/// One note of a cue, in milliseconds from the cue's start, gliding from one frequency to another.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Note {
    pub start: u32,
    pub length: u32,
    pub from: f32,
    pub to: f32,
}

/// How a cue is synthesized.
#[derive(Clone, Debug, PartialEq)]
pub struct Spec {
    pub notes: Vec<Note>,
    /// Each partial's frequency as a multiple of the note's, and its amplitude.
    pub partials: &'static [(f32, f32)],
    /// Each note's attack, in milliseconds; it then decays exponentially to silence at its end.
    pub attack: u32,
    /// The cue's peak, as a fraction of full scale: well below speech, which peaks near full scale.
    pub level: f32,
}

impl Spec {
    /// How long the cue lasts, in milliseconds.
    pub fn length(&self) -> u32 {
        self.notes
            .iter()
            .map(|note| note.start + note.length)
            .max()
            .unwrap_or(0)
    }

    /// The cue's samples at `sample_rate`, in `i16` units, peaking at [`Self::level`] of full scale. Deterministic.
    pub fn render(&self, sample_rate: u32) -> Vec<f32> {
        let rate = f64::from(sample_rate);
        let samples =
            |milliseconds: u32| (f64::from(milliseconds) * rate / 1000.0).round() as usize;
        let mut output = vec![0.0_f64; samples(self.length())];
        for note in &self.notes {
            let start = samples(note.start);
            let length = samples(note.length).max(1);
            let attack = samples(self.attack).clamp(1, length);
            // Decays to −60 dB at the note's end.
            let decay = (1000.0_f64).ln() / (length - attack).max(1) as f64;
            let mut phase = 0.0_f64;
            for index in 0..length {
                let progress = index as f64 / length as f64;
                let frequency = f64::from(note.from) + f64::from(note.to - note.from) * progress;
                phase += frequency / rate;
                let envelope = if index < attack {
                    index as f64 / attack as f64
                } else {
                    (-decay * (index - attack) as f64).exp()
                };
                // The last couple of milliseconds fade to zero, so that the cue never ends on a step.
                let tail = ((length - index) as f64 / (rate * 0.002)).min(1.0);
                let value: f64 = self
                    .partials
                    .iter()
                    .map(|(ratio, amplitude)| {
                        f64::from(*amplitude) * (TAU * phase * f64::from(*ratio)).sin()
                    })
                    .sum();
                if let Some(sample) = output.get_mut(start + index) {
                    *sample += value * envelope * tail;
                }
            }
        }
        let peak = output
            .iter()
            .fold(0.0_f64, |peak, sample| peak.max(sample.abs()));
        let scale = if peak > 0.0 {
            f64::from(self.level) * f64::from(i16::MAX) / peak
        } else {
            0.0
        };
        output
            .into_iter()
            .map(|sample| (sample * scale) as f32)
            .collect()
    }
}

/// Every cue rendered at one device rate.
pub struct Bank {
    sample_rate: u32,
    sounds: Vec<Vec<f32>>,
}

impl Bank {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            sample_rate,
            sounds: Earcon::ALL
                .iter()
                .map(|earcon| earcon.spec().render(sample_rate))
                .collect(),
        }
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn samples(&self, earcon: Earcon) -> &[f32] {
        &self.sounds[earcon.index()]
    }
}

#[cfg(test)]
mod tests;
