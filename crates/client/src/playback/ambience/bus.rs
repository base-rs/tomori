// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The ambience bus's voices, in the audio callback: each takes the blocks its channel holds, without waiting, plays
//! them at the device's rate, at its pitch, in a straight line between samples, and places them as a voice is placed,
//! a bed alike in both ears; and gives each block back once played, so that nothing is allocated or freed here. A
//! voice whose next block has not come yet is silent until it comes. See `design/spatial-audio.md`.
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError};

use super::VOICES;
use super::engine::{Block, Kind, Spec};
use crate::playback::Layout;
use crate::playback::spatial::{self, Ears, Placement, Scene};

/// How much of a loop's radius it fades over, to nothing at the radius.
const FADE: f32 = 0.25;

/// The rate the voices' ears are made for until the bus is prepared for the device's.
const DEFAULT_RATE: u32 = 48_000;

/// What a voice's next frame is.
enum Pull {
    Frame([f32; 2]),
    /// Its next block has not come yet.
    Late,
    /// Its sound has ended, or the thread that fed it has gone.
    End,
}

struct Voice {
    blocks: Receiver<Block>,
    spec: Option<Spec>,
    block: Option<Vec<f32>>,
    /// How far through `block`, in samples.
    at: usize,
    /// The frames either side of where it is, and how far it is from the first to the second.
    previous: [f32; 2],
    next: [f32; 2],
    t: f64,
    ears: Ears,
}

impl Voice {
    fn new(blocks: Receiver<Block>, rate: u32) -> Self {
        Self {
            blocks,
            spec: None,
            block: None,
            at: 0,
            previous: [0.0; 2],
            next: [0.0; 2],
            t: 0.0,
            ears: Ears::new(rate),
        }
    }

    /// Begin the sound in its channel, if one has begun.
    fn poll(&mut self) {
        match self.blocks.try_recv() {
            Ok(Block::Begin(spec)) => {
                self.spec = Some(spec);
                self.previous = [0.0; 2];
                self.next = [0.0; 2];
                // Two frames are taken before the first is heard.
                self.t = 2.0;
            }
            Ok(_) => unreachable!("a sound's blocks follow its beginning, and its end is its last"),
            Err(_) => {}
        }
    }

    /// Its next frame, in `channels`.
    fn pull(&mut self, channels: usize, recycle: &SyncSender<Vec<f32>>) -> Pull {
        loop {
            if let Some(block) = &self.block
                && self.at + channels <= block.len()
            {
                let frame = [block[self.at], block[self.at + channels - 1]];
                self.at += channels;
                return Pull::Frame(frame);
            }
            if let Some(block) = self.block.take() {
                // Room was made for every block, so this frees nothing.
                let _ = recycle.try_send(block);
            }
            match self.blocks.try_recv() {
                Ok(Block::Samples(block)) => {
                    self.block = Some(block);
                    self.at = 0;
                }
                Ok(Block::Begin(_)) => unreachable!("a sound ends before the next begins"),
                Ok(Block::End) | Err(TryRecvError::Disconnected) => return Pull::End,
                Err(TryRecvError::Empty) => return Pull::Late,
            }
        }
    }

    /// Add its next `frames` into `out`, in `layout`, placed in `scene`, at the device's `rate`. Returns whether its
    /// next block was late.
    fn mix(
        &mut self,
        scene: &Scene,
        rate: u32,
        layout: Layout,
        out: &mut [f32],
        scratch: &mut Vec<f32>,
        recycle: &SyncSender<Vec<f32>>,
    ) -> bool {
        if self.spec.is_none() {
            self.poll();
        }
        let Some(spec) = self.spec else {
            return false;
        };
        let channels = spec.channels();
        let frames = out.len() / layout.channels();
        scratch.clear();
        scratch.resize(frames * channels, 0.0);
        let step = f64::from(spec.rate) * f64::from(spec.pitch) / f64::from(rate.max(1));
        let (mut late, mut ended) = (false, false);
        'frames: for frame in scratch.chunks_exact_mut(channels) {
            while self.t >= 1.0 {
                match self.pull(channels, recycle) {
                    Pull::Frame(next) => {
                        self.previous = self.next;
                        self.next = next;
                        self.t -= 1.0;
                    }
                    Pull::Late => {
                        late = true;
                        break 'frames;
                    }
                    Pull::End => {
                        ended = true;
                        break 'frames;
                    }
                }
            }
            let t = self.t as f32;
            for (channel, sample) in frame.iter_mut().enumerate() {
                let (from, to) = (self.previous[channel], self.next[channel]);
                *sample = from + (to - from) * t;
            }
            self.t += step;
        }
        let mono = scene.mono || layout == Layout::Mono;
        let place = |position| match scene.listener {
            Some(listener) => {
                let placed = spatial::place(&listener, position, 0.0, rate, mono);
                let offset = std::array::from_fn::<f32, 3, _>(|axis| {
                    position[axis] - listener.position[axis]
                });
                (
                    placed,
                    offset.iter().map(|axis| axis * axis).sum::<f32>().sqrt(),
                )
            }
            None => (Placement::CENTRE, 0.0),
        };
        let placement = match spec.kind {
            Kind::Bed => None,
            Kind::Loop { path, radius } => {
                let nearest = scene
                    .listener
                    .map_or(path.points()[0], |listener| path.nearest(listener.position));
                let (placement, distance) = place(nearest);
                let radius = radius.max(0.01);
                let fade = ((radius - distance) / (radius * FADE)).clamp(0.0, 1.0);
                Some(placement.scaled(fade * spec.gain))
            }
            Kind::Call { position } => Some(place(position).0.scaled(spec.gain)),
        };
        match (placement, layout) {
            (Some(placement), Layout::Stereo) => {
                self.ears.render(scratch, placement, (1.0, 1.0), out);
            }
            (Some(placement), Layout::Mono) => {
                self.ears.render_mono(scratch, placement, (1.0, 1.0), out);
            }
            (None, Layout::Stereo) => {
                for (total, pair) in out
                    .as_chunks_mut::<2>()
                    .0
                    .iter_mut()
                    .zip(scratch.as_chunks::<2>().0)
                {
                    // Mono audio hears a bed's two channels together in both ears.
                    let [left, right] = if scene.mono {
                        let both = (pair[0] + pair[1]) * 0.5;
                        [both, both]
                    } else {
                        *pair
                    };
                    total[0] += left * spec.gain;
                    total[1] += right * spec.gain;
                }
            }
            (None, Layout::Mono) => {
                for (total, pair) in out.iter_mut().zip(scratch.as_chunks::<2>().0) {
                    *total += (pair[0] + pair[1]) * 0.5 * spec.gain;
                }
            }
        }
        // Its last block was given back as its end was taken.
        if ended {
            self.spec = None;
        }
        late
    }
}

/// The voices of the ambience, which its thread feeds, in the audio callback.
pub(crate) struct Bus {
    voices: Vec<Voice>,
    recycle: SyncSender<Vec<f32>>,
    rate: u32,
    late: u64,
}

impl Bus {
    pub fn new(receivers: Vec<Receiver<Block>>, recycle: SyncSender<Vec<f32>>) -> Self {
        debug_assert_eq!(receivers.len(), VOICES);
        Self {
            voices: receivers
                .into_iter()
                .map(|blocks| Voice::new(blocks, DEFAULT_RATE))
                .collect(),
            recycle,
            rate: DEFAULT_RATE,
            late: 0,
        }
    }

    /// Place for the device's `rate`, which makes each voice's ears again only if it changed.
    pub fn prepare(&mut self, rate: u32) {
        if rate != self.rate {
            self.rate = rate;
            for voice in &mut self.voices {
                voice.ears = Ears::new(rate);
            }
        }
    }

    /// How many voices are playing a sound.
    pub fn playing(&self) -> usize {
        self.voices
            .iter()
            .filter(|voice| voice.spec.is_some())
            .count()
    }

    /// How many times a voice's next block was late.
    pub fn late(&self) -> u64 {
        self.late
    }

    /// Add every voice's next samples into `out`, interleaved in `layout`, placed in `scene`, mixing each in `scratch`.
    pub fn mix(&mut self, scene: &Scene, layout: Layout, out: &mut [f32], scratch: &mut Vec<f32>) {
        for voice in &mut self.voices {
            if voice.mix(scene, self.rate, layout, out, scratch, &self.recycle) {
                self.late += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests;
