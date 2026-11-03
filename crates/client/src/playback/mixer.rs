// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use std::collections::HashMap;
use std::time::Duration;

use protocol::{AGENT_SOURCE, AudioFrame, Gate, REPLAY_SOURCE};

use super::{Bank, Earcon, JitterBuffer, JitterOptions, Limiter, PlaybackStatistics};

/// How long a source may go without frames before its buffer is released.
const IDLE_LIMIT: Duration = Duration::from_secs(10);
/// How quickly a source's level falls once it plays more quietly: it drops to about a third over this time.
const LEVEL_RELEASE: Duration = Duration::from_millis(150);
/// How long playback takes to fade out when silenced, and back in afterwards, so that it does not click.
const FADE: Duration = Duration::from_millis(10);
/// The most sound cues that play at once; another replaces the one furthest through.
const VOICES: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MixerOptions {
    /// The output device rate.
    pub sample_rate: u32,
    pub jitter: JitterOptions,
}

impl Default for MixerOptions {
    fn default() -> Self {
        Self {
            sample_rate: 48_000,
            jitter: JitterOptions::default(),
        }
    }
}

/// One buffered source and the level the mixer last measured for it, kept together so that [`Mixer::mix`] never inserts into a map on the audio thread.
struct Source {
    buffer: JitterBuffer,
    /// See [`Mixer::source_level`].
    level: f32,
}

impl Source {
    fn new(sample_rate: u32, options: JitterOptions) -> Self {
        Self {
            buffer: JitterBuffer::new(sample_rate, options),
            level: 0.0,
        }
    }
}

/// A sound cue playing, and how far through it is.
#[derive(Clone, Copy, Debug)]
struct Voice {
    earcon: Earcon,
    position: usize,
}

/// Mixes every remote participant and the agent speech stream into one mono `i16` stream at the device rate, with this client's own sound cues.
///
/// Feed it every received [`AudioFrame`] with [`Self::push`] and pull output from the audio callback with [`Self::mix`]. Each source has its own [`JitterBuffer`], so one late or bursty participant does not delay the others. Agent volume and mute are local to this mixer, and apply to agent speech played again from [`REPLAY_SOURCE`] as to live agent speech; muting still consumes agent audio in real time, so unmuting resumes live speech rather than replaying what was missed.
///
/// Sound cues ([`Self::play`]) are local: never sent, never a source, never muted with agent speech. They are rendered once when the mixer is created, so playing one allocates nothing. Silencing ([`Self::set_silenced`]) fades out everything but the cues that bypass it, then the limiter comes last.
pub struct Mixer {
    options: MixerOptions,
    sources: HashMap<u32, Source>,
    /// This client's own participant number, whose audio is never played back.
    local_source: Option<u32>,
    agent_volume: f32,
    agent_muted: bool,
    limiter: Limiter,
    accumulator: Vec<f32>,
    /// One source's contribution to the current [`Self::mix`] call, so its level can be measured.
    scratch: Vec<f32>,
    statistics: PlaybackStatistics,
    bank: Bank,
    voices: [Option<Voice>; VOICES],
    /// The cues that play through silencing, for the current [`Self::mix`] call.
    bypass: Vec<f32>,
    silenced: bool,
    /// How far playback has faded out, in samples: 0 is full volume, and the fade length is silent.
    faded: u32,
}

impl Mixer {
    pub fn new(options: MixerOptions) -> Self {
        assert!(
            options.sample_rate > 0,
            "device sample rate must be positive"
        );
        Self {
            options,
            sources: HashMap::new(),
            local_source: None,
            agent_volume: 1.0,
            agent_muted: false,
            limiter: Limiter::new(options.sample_rate),
            accumulator: Vec::new(),
            scratch: Vec::new(),
            statistics: PlaybackStatistics::default(),
            bank: Bank::new(options.sample_rate),
            voices: [None; VOICES],
            bypass: Vec::new(),
            silenced: false,
            faded: 0,
        }
    }

    /// Play a sound cue from its start. When [`VOICES`] already play, it replaces the one furthest through.
    pub fn play(&mut self, earcon: Earcon) {
        let voice = Some(Voice {
            earcon,
            position: 0,
        });
        let slot = match self.voices.iter().position(Option::is_none) {
            Some(free) => free,
            None => (0..VOICES)
                .max_by_key(|&index| self.voices[index].map_or(0, |voice| voice.position))
                .expect("there are voices"),
        };
        self.voices[slot] = voice;
    }

    /// How many sound cues are playing.
    pub fn playing_cues(&self) -> usize {
        self.voices.iter().flatten().count()
    }

    /// Silence playback, fading out over [`FADE`], while still consuming what arrives in real time; cues that [`Earcon::bypasses_silence`] still play. Used while the microphone records push-to-talk; see `design/live-audio.md`.
    pub fn set_silenced(&mut self, silenced: bool) {
        self.silenced = silenced;
    }

    /// Silence playback at once, without fading out, as for a mixer made while playback is already silenced.
    pub fn silence_now(&mut self) {
        self.silenced = true;
        self.faded = (FADE.as_secs_f64() * f64::from(self.options.sample_rate))
            .ceil()
            .max(1.0) as u32;
    }

    pub fn sample_rate(&self) -> u32 {
        self.options.sample_rate
    }

    /// Ignore audio from this participant number: a client never hears its own microphone. Set from [`protocol::ServerMessage::Welcome`].
    pub fn set_local_source(&mut self, source: Option<u32>) {
        self.local_source = source;
        if let Some(source) = source {
            self.clear_source(source);
        }
    }

    pub fn agent_volume(&self) -> f32 {
        self.agent_volume
    }

    /// Set the local agent speech gain. Negative values are treated as zero.
    pub fn set_agent_volume(&mut self, volume: f32) {
        self.agent_volume = volume.max(0.0);
    }

    pub fn is_agent_muted(&self) -> bool {
        self.agent_muted
    }

    pub fn set_agent_muted(&mut self, muted: bool) {
        self.agent_muted = muted;
    }

    pub fn statistics(&self) -> PlaybackStatistics {
        self.statistics
    }

    /// The sources currently buffered, including those still filling.
    pub fn sources(&self) -> impl Iterator<Item = u32> + '_ {
        self.sources.keys().copied()
    }

    /// Discard all buffered audio. The application owns the mixer, so it calls this when the audio it hears changes context: on [`protocol::ServerMessage::Left`], before joining a different space, and after reconnecting. Sound cues keep playing: they are this client's own, such as the click as it leaves a space.
    pub fn clear(&mut self) {
        self.sources.clear();
    }

    /// Discard one source's buffered audio, for example when a participant leaves the space. Its next frame starts a new buffer.
    pub fn clear_source(&mut self, source: u32) {
        self.sources.remove(&source);
    }

    /// Whether `source` is contributing audio now, rather than buffering, stalled or absent. Agent speech that is muted still counts as playing, because it is consumed in real time.
    pub fn is_source_playing(&self, source: u32) -> bool {
        self.sources
            .get(&source)
            .is_some_and(|source| source.buffer.is_playing())
    }

    /// Whether `source` has played or buffered audio within `within`: true through the short silences of a stream, such as the breath between spoken segments, when [`Self::is_source_playing`] is false.
    pub fn is_source_recent(&self, source: u32, within: Duration) -> bool {
        let within = (within.as_secs_f64() * f64::from(self.options.sample_rate)) as u64;
        self.sources.get(&source).is_some_and(|source| {
            source.buffer.is_playing()
                || source.buffer.buffered_samples() > 0
                || source.buffer.idle_samples() < within
        })
    }

    /// The recent loudness of what [`Self::mix`] played from `source`, before its gain, so that muted agent speech still has a level, from 0 (silent or absent) to 1 (full scale), for visuals such as a speaking indicator. It follows a louder signal at once and falls over about [`LEVEL_RELEASE`] when the source goes quiet.
    pub fn source_level(&self, source: u32) -> f32 {
        self.sources.get(&source).map_or(0.0, |source| source.level)
    }

    /// Queue a received frame for playback.
    pub fn push(&mut self, frame: AudioFrame) {
        if frame.gate == Gate::Off || Some(frame.source) == self.local_source {
            return;
        }
        let source = self
            .sources
            .entry(frame.source)
            .or_insert_with(|| Source::new(frame.sample_rate, self.options.jitter));
        if source.buffer.sample_rate() != frame.sample_rate {
            // A new speech segment can change rate; restart the source rather than play mismatched audio.
            *source = Source::new(frame.sample_rate, self.options.jitter);
        }
        source.buffer.push(frame, &mut self.statistics);
    }

    /// Fill `output` with the next mixed samples. Never blocks: missing audio is silence.
    pub fn mix(&mut self, output: &mut [i16]) {
        self.accumulator.clear();
        self.accumulator.resize(output.len(), 0.0);
        let rate = self.options.sample_rate;
        let agent_gain = if self.agent_muted {
            0.0
        } else {
            self.agent_volume
        };
        let release =
            (-(output.len() as f64) / (LEVEL_RELEASE.as_secs_f64() * f64::from(rate))).exp() as f32;
        for (&id, source) in self.sources.iter_mut() {
            let gain = if id == AGENT_SOURCE || id == REPLAY_SOURCE {
                agent_gain
            } else {
                1.0
            };
            // Mixed at unity gain and measured before the gain is applied, so muted agent speech still shows as speaking.
            self.scratch.clear();
            self.scratch.resize(output.len(), 0.0);
            source
                .buffer
                .mix_into(&mut self.scratch, rate, 1.0, &mut self.statistics);
            let mut energy = 0.0;
            for (total, &sample) in self.accumulator.iter_mut().zip(&self.scratch) {
                *total += sample * gain;
                energy += sample * sample;
            }
            let rms = (energy / output.len().max(1) as f32).sqrt() / f32::from(i16::MAX);
            source.level = rms.min(1.0).max(source.level * release);
        }
        let idle_limit = (IDLE_LIMIT.as_secs_f64() * f64::from(rate)) as u64;
        self.sources.retain(|_, source| {
            source.buffer.is_playing()
                || source.buffer.buffered_samples() > 0
                || source.buffer.idle_samples() < idle_limit
        });
        self.bypass.clear();
        self.bypass.resize(output.len(), 0.0);
        for slot in &mut self.voices {
            let Some(voice) = slot else {
                continue;
            };
            let samples = &self.bank.samples(voice.earcon)[voice.position..];
            let target = if voice.earcon.bypasses_silence() {
                &mut self.bypass
            } else {
                &mut self.accumulator
            };
            for (total, sample) in target.iter_mut().zip(samples) {
                *total += sample;
            }
            voice.position += samples.len().min(output.len());
            if voice.position >= self.bank.samples(voice.earcon).len() {
                *slot = None;
            }
        }
        let length = (FADE.as_secs_f64() * f64::from(rate)).ceil().max(1.0) as u32;
        if self.silenced || self.faded > 0 {
            for sample in self.accumulator.iter_mut() {
                self.faded = if self.silenced {
                    (self.faded + 1).min(length)
                } else {
                    self.faded.saturating_sub(1)
                };
                *sample *= 1.0 - self.faded as f32 / length as f32;
            }
        }
        for (total, sample) in self.accumulator.iter_mut().zip(&self.bypass) {
            *total += sample;
        }
        self.limiter.process(&self.accumulator, output);
    }
}

#[cfg(test)]
mod tests;
