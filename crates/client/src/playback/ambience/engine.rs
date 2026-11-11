// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The ambience's own thread: it loads the slots of the ambience it is given, starts its beds and loops, schedules its
//! calls, and decodes each voice's sound a block at a time into a bounded channel, which the audio callback reads
//! without waiting or allocating, its blocks coming back to be filled again. Off, or without an ambience, it decodes
//! nothing and waits. See `design/spatial-audio.md`.
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::time::{Duration, Instant};

use super::bus::Bus;
use super::decoder::Decoder;
use super::{Ambience, MOST_BYTES, Path, Scheduler, Slot, VOICES};
use crate::playback::spatial::Position;

/// The frames in each block.
pub(crate) const BLOCK: usize = 1_024;
/// The blocks each voice's channel holds, ahead of the callback: at 48 kHz, about 128 ms of sound.
pub(crate) const DEPTH: usize = 6;
/// How often the thread decodes more while it plays, well within what [`DEPTH`] holds.
pub(crate) const TICK: Duration = Duration::from_millis(10);
/// The blocks there are, made as the thread starts: as many as every voice holds at most, [`DEPTH`] in its channel,
/// one the callback plays and one waiting to be sent.
pub(crate) const BLOCKS: usize = VOICES * (DEPTH + 2);

/// How a voice's sound is heard.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Kind {
    /// In stereo, alike wherever the listener is.
    Bed,
    /// From the point of `path` nearest the listener, fading to nothing at `radius`.
    Loop { path: Path, radius: f32 },
    /// From `position`.
    Call { position: Position },
}

/// A sound beginning in a voice: how it is heard, its samples a second, the rate it plays at and its gain.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Spec {
    pub kind: Kind,
    pub rate: u32,
    pub pitch: f32,
    pub gain: f32,
}

impl Spec {
    /// Its channels: two for a bed, one otherwise.
    pub fn channels(&self) -> usize {
        match self.kind {
            Kind::Bed => 2,
            _ => 1,
        }
    }
}

/// What the thread sends a voice, in order: a sound beginning, its samples, interleaved, and its end.
#[derive(Debug)]
pub(crate) enum Block {
    Begin(Spec),
    Samples(Vec<f32>),
    End,
}

/// What the application tells the thread.
pub(crate) enum Command {
    /// Play this ambience, or none.
    Ambience(Option<Arc<Ambience>>),
    /// The participant's level, as a gain: at 0, nothing is decoded.
    Gain(f32),
}

/// A voice the thread is filling: what it plays and its decoder, until its end has been sent.
struct Playing {
    /// `None` once its sound has ended, with its end still to send.
    decoder: Option<Decoder>,
    channels: usize,
    /// The block its channel had no room for, sent first next time.
    pending: Option<Block>,
}

/// The ambience being played: its slots' files, loaded once, and its calls' schedule.
struct Prepared {
    ambience: Arc<Ambience>,
    beds: Vec<Option<Arc<[u8]>>>,
    loops: Vec<Option<Arc<[u8]>>>,
    calls: Vec<Vec<Option<Arc<[u8]>>>>,
    scheduler: Scheduler,
    /// Whether each bed and loop has been given a voice, which it keeps.
    started: Vec<bool>,
    /// Whether any of its slots has a sound to play.
    audible: bool,
}

/// Load `slot`'s file, saying once why it is silent if it is.
fn load(slot: &Slot) -> Option<Arc<[u8]>> {
    let name = slot.name();
    let Some(bytes) = slot.load() else {
        log::info!("Ambience: {name} has no sound yet, so it is silent.");
        return None;
    };
    if bytes.len() > MOST_BYTES {
        log::warn!(
            "Ambience: {name} is {} bytes, more than the {MOST_BYTES} a sound may be, so it is silent.",
            bytes.len()
        );
        return None;
    }
    match Decoder::new(Arc::from(bytes.clone()), false) {
        Ok(_) => Some(Arc::from(bytes)),
        Err(error) => {
            log::warn!("Ambience: {name} is not Ogg Vorbis ({error}), so it is silent.");
            None
        }
    }
}

/// The thread's side of the ambience: what it plays, each voice's channel and the blocks it fills.
pub(crate) struct Feeder {
    senders: Vec<SyncSender<Block>>,
    recycled: Receiver<Vec<f32>>,
    pool: Vec<Vec<f32>>,
    voices: Vec<Option<Playing>>,
    prepared: Option<Prepared>,
    gain: f32,
    seed: u64,
}

/// A feeder and the bus it feeds, with `seed` for its calls.
pub(crate) fn pair(seed: u64) -> (Feeder, Bus) {
    let mut senders = Vec::with_capacity(VOICES);
    let mut receivers = Vec::with_capacity(VOICES);
    for _ in 0..VOICES {
        let (sender, receiver) = mpsc::sync_channel(DEPTH);
        senders.push(sender);
        receivers.push(receiver);
    }
    // As many blocks as every voice can hold at once, those in its channel, the one the callback plays and the one
    // waiting to be sent, and never more, so that the callback always has room to give back every one it took and so
    // never frees one.
    let blocks = BLOCKS;
    let (recycle, recycled) = mpsc::sync_channel(blocks);
    let feeder = Feeder {
        senders,
        recycled,
        pool: (0..blocks).map(|_| Vec::with_capacity(BLOCK * 2)).collect(),
        voices: (0..VOICES).map(|_| None).collect(),
        prepared: None,
        gain: 0.0,
        seed,
    };
    (feeder, Bus::new(receivers, recycle))
}

impl Feeder {
    /// Take `command` at `now`, on the thread's clock. A new ambience ends every voice of the last.
    pub fn apply(&mut self, command: Command, now: Duration) {
        match command {
            Command::Ambience(ambience) => {
                self.end_all();
                self.prepared = ambience.map(|ambience| {
                    let calls: Vec<Vec<_>> = ambience
                        .calls
                        .iter()
                        .map(|call| call.variants.iter().map(load).collect())
                        .collect();
                    let beds: Vec<_> = ambience.beds.iter().map(|bed| load(&bed.slot)).collect();
                    let loops: Vec<_> = ambience
                        .loops
                        .iter()
                        .map(|sound| load(&sound.slot))
                        .collect();
                    let audible = beds
                        .iter()
                        .chain(&loops)
                        .chain(calls.iter().flatten())
                        .any(Option::is_some);
                    Prepared {
                        beds,
                        loops,
                        calls,
                        audible,
                        scheduler: Scheduler::new(&ambience.calls, self.seed, now),
                        started: vec![false; ambience.beds.len() + ambience.loops.len()],
                        ambience,
                    }
                });
                self.seed = self.seed.wrapping_add(1);
            }
            Command::Gain(gain) => {
                let off = gain <= 0.0;
                if off && self.gain > 0.0 {
                    self.end_all();
                    // Begun again from the start once heard again.
                    if let Some(prepared) = &mut self.prepared {
                        prepared.started.fill(false);
                    }
                }
                self.gain = gain;
            }
        }
    }

    /// End every voice: what is waiting to be sent of it goes back to be filled again, and its end is sent next.
    fn end_all(&mut self) {
        for playing in self.voices.iter_mut().flatten() {
            playing.decoder = None;
            if let Some(Block::Samples(block)) = playing.pending.take() {
                self.pool.push(block);
            }
            playing.pending = Some(Block::End);
        }
    }

    /// Whether there is nothing to decode or send: off, or without an ambience with a sound in it, with every voice's
    /// end sent.
    pub fn idle(&self) -> bool {
        let audible = self
            .prepared
            .as_ref()
            .is_some_and(|prepared| prepared.audible);
        (self.gain <= 0.0 || !audible) && self.voices.iter().all(Option::is_none)
    }

    /// How many voices are playing or ending.
    #[cfg(test)]
    pub fn playing(&self) -> usize {
        self.voices.iter().flatten().count()
    }

    /// Begin a sound of `kind` in a free voice, from `bytes`, at `pitch` and `gain`, unless every voice plays: a call
    /// once, and a bed or loop over and over. Returns whether it began.
    fn begin(&mut self, bytes: &Arc<[u8]>, kind: Kind, pitch: f32, gain: f32) -> bool {
        let Some(free) = self.voices.iter().position(Option::is_none) else {
            return false;
        };
        let looping = !matches!(kind, Kind::Call { .. });
        let decoder = match Decoder::new(bytes.clone(), looping) {
            Ok(decoder) => decoder,
            Err(_) => unreachable!("its bytes opened as they loaded"),
        };
        let spec = Spec {
            kind,
            rate: decoder.rate(),
            pitch,
            gain,
        };
        self.voices[free] = Some(Playing {
            channels: spec.channels(),
            decoder: Some(decoder),
            pending: Some(Block::Begin(spec)),
        });
        true
    }

    /// Start what is due by `now`, and decode into every voice's channel as far as it has room.
    pub fn pump(&mut self, now: Duration) {
        while let Ok(block) = self.recycled.try_recv() {
            self.pool.push(block);
        }
        if self.gain > 0.0
            && let Some(mut prepared) = self.prepared.take()
        {
            self.start(&mut prepared, now);
            self.prepared = Some(prepared);
        }
        for index in 0..VOICES {
            self.fill(index);
        }
    }

    /// Give each bed and loop not yet playing a voice, in order, and begin each call due, skipping those due while
    /// every voice plays or whose variant is silent.
    fn start(&mut self, prepared: &mut Prepared, now: Duration) {
        let ambience = prepared.ambience.clone();
        for (index, bed) in ambience.beds.iter().enumerate() {
            if let Some(bytes) = &prepared.beds[index]
                && !prepared.started[index]
            {
                prepared.started[index] = self.begin(bytes, Kind::Bed, 1.0, super::gain(bed.gain));
            }
        }
        let beds = ambience.beds.len();
        for (index, sound) in ambience.loops.iter().enumerate() {
            if let Some(bytes) = &prepared.loops[index]
                && !prepared.started[beds + index]
            {
                let kind = Kind::Loop {
                    path: sound.path,
                    radius: sound.radius,
                };
                prepared.started[beds + index] =
                    self.begin(bytes, kind, 1.0, super::gain(sound.gain));
            }
        }
        while let Some(due) = prepared.scheduler.due(&ambience.calls, now) {
            let Some(Some(bytes)) = prepared.calls[due.call].get(due.variant) else {
                continue;
            };
            let kind = Kind::Call {
                position: due.position,
            };
            if !self.begin(bytes, kind, due.pitch.max(0.01), due.gain) {
                log::debug!("Ambience: a call came while every voice played, so it is skipped.");
            }
        }
    }

    /// Send voice `index` what it has waiting, then decode into it until its channel is full or its sound ends.
    fn fill(&mut self, index: usize) {
        loop {
            let Some(playing) = self.voices[index].as_mut() else {
                return;
            };
            let block = match playing.pending.take() {
                Some(block) => block,
                None => {
                    let Some(decoder) = playing.decoder.as_mut() else {
                        unreachable!(
                            "a voice without a decoder has its end waiting until it is sent"
                        );
                    };
                    // Never a block more than there are, which the callback could find no room to give back: one
                    // given back since the thread looked is taken, and with none, as cannot be, it waits.
                    let Some(mut block) = self.pool.pop().or_else(|| self.recycled.try_recv().ok())
                    else {
                        return;
                    };
                    block.clear();
                    if decoder.read(&mut block, BLOCK, playing.channels) == 0 {
                        self.pool.push(block);
                        playing.decoder = None;
                        Block::End
                    } else {
                        Block::Samples(block)
                    }
                }
            };
            let end = matches!(block, Block::End);
            match self.senders[index].try_send(block) {
                Ok(()) if end => {
                    self.voices[index] = None;
                    return;
                }
                Ok(()) => {}
                Err(TrySendError::Full(block)) => {
                    playing.pending = Some(block);
                    return;
                }
                // The bus has gone with its player: nothing hears this voice.
                Err(TrySendError::Disconnected(_)) => {
                    self.voices[index] = None;
                    return;
                }
            }
        }
    }
}

/// Run `feeder` on this thread, taking `commands`, until they end: decoding every [`TICK`] while there is something to
/// decode or send, and otherwise waiting for the next command, so that nothing is decoded while ambience is off.
pub(crate) fn run(mut feeder: Feeder, commands: Receiver<Command>) {
    let epoch = Instant::now();
    loop {
        let next = if feeder.idle() {
            commands.recv().map(Some).map_err(drop)
        } else {
            match commands.recv_timeout(TICK) {
                Ok(command) => Ok(Some(command)),
                Err(RecvTimeoutError::Timeout) => Ok(None),
                Err(RecvTimeoutError::Disconnected) => Err(()),
            }
        };
        let Ok(command) = next else {
            return;
        };
        let now = epoch.elapsed();
        if let Some(command) = command {
            feeder.apply(command, now);
        }
        while let Ok(command) = commands.try_recv() {
            feeder.apply(command, now);
        }
        feeder.pump(now);
    }
}

#[cfg(test)]
pub(crate) mod tests;
