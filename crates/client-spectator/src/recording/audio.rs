// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The recording's sound: the mix the spectator's player renders, as the speaker is given it, copied to a stream of 48 kHz stereo samples timed by the recording's clock. See `design/spectator.md`.
//!
//! The player's mix is made by `tomori-client`'s mixer, so the voices it plays, the agent's speech, the ambience and the ducking under them are those the spectator plays, and a recording hears what the room does. A [`tap`](tomori_client::playback::Player::tap) on the player copies each block the audio callback renders: the speaker plays it exactly as it would without one. When the spectator does not play to the speaker while it records, or its speaker cannot be opened, [`Tee`] renders the player itself, at 48 kHz, on a thread paced by the clock.
//!
//! Blocks come from a device's callbacks, at its rate and with jitter, so [`Aligner`] puts them on the clock: silence fills the gap before the first block, a block that was lost, and any time the player was not rendered; a block that comes early is trimmed; and a device whose rate is not 48 kHz is resampled.
use super::{CHANNELS, Origin, SAMPLE_RATE, Stop, clock::FrameClock, system, video::Ending};
use std::{
    io::Write,
    sync::{
        Arc, PoisonError,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use tomori_client::{
    playback::{Layout, Tap},
    speaker::{Failed, Output, Playback, Route, Shared},
};

/// How far the audio may be from the clock before it is put right: 20 ms, which is longer than a device's callbacks vary.
const TOLERANCE: i64 = SAMPLE_RATE as i64 / 50;
/// How far behind the clock the audio may fall with nothing arriving before it is filled with silence: 100 ms.
const IDLE: i64 = SAMPLE_RATE as i64 / 10;
/// How long a block of the player's own rendering lasts.
const BLOCK: Duration = Duration::from_millis(10);
/// How much of the sound the writer holds back, beyond a frame's length: 30 ms, so that the end of the recording, which is rounded to a frame, can cut it to the length of the video.
const HELD: Duration = Duration::from_millis(30);
/// How many blocks wait for the writer before the tap drops them.
const QUEUED: usize = 2048;

/// A block the player rendered: when its render ended on the recording's clock, and what it rendered.
pub struct Block {
    pub at: Duration,
    pub rate: u32,
    pub layout: Layout,
    pub samples: Vec<i16>,
}

/// Copies what the player renders to the audio writer, from the audio callback, without waiting for it.
#[derive(Clone)]
pub struct AudioTap {
    sender: SyncSender<Block>,
    origin: Origin,
    closed: Arc<AtomicBool>,
    lost: Arc<AtomicU64>,
}

impl AudioTap {
    /// The tap and the blocks it sends, for a recording that begins at `origin`; it copies nothing before then.
    pub fn new(origin: Origin) -> (Self, Receiver<Block>) {
        let (sender, blocks) = mpsc::sync_channel(QUEUED);
        let tap = Self {
            sender,
            origin,
            closed: Arc::default(),
            lost: Arc::default(),
        };
        (tap, blocks)
    }

    /// The tap for a player. It allocates a block for each callback, as the player does for each frame it receives, and drops the block, counted, if the writer has fallen far behind.
    pub fn tap(&self) -> Tap {
        let tap = self.clone();
        Box::new(move |rate, layout, samples| {
            if tap.closed.load(Ordering::Relaxed) || !tap.origin.begun() {
                return;
            }
            let block = Block {
                at: tap.origin.elapsed(),
                rate,
                layout,
                samples: samples.to_vec(),
            };
            match tap.sender.try_send(block) {
                Ok(()) => {}
                Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => {
                    tap.lost.fetch_add(1, Ordering::Relaxed);
                }
            }
        })
    }

    /// Stop copying: the recording is ending.
    pub fn close(&self) {
        self.closed.store(true, Ordering::Relaxed);
    }

    /// How many blocks were dropped because the writer could not keep up.
    pub fn lost(&self) -> u64 {
        self.lost.load(Ordering::Relaxed)
    }
}

/// Interpolates a stereo stream from one rate to another, a block at a time, with a cubic through four neighbouring frames. It is for a device that does not run at 48 kHz, and keeps what it needs between blocks.
struct Resampler {
    /// Input frames for each output frame.
    step: f64,
    /// Where the next output frame is among `frames`.
    position: f64,
    /// The input frames not yet passed, led by one before the first.
    frames: Vec<[f32; 2]>,
}

impl Resampler {
    fn new(from: u32, to: u32) -> Self {
        Self {
            step: f64::from(from) / f64::from(to),
            position: 1.0,
            frames: vec![[0.0; 2]],
        }
    }

    /// Resample `input`, interleaved stereo, appending the output, interleaved, to `output`.
    fn push(&mut self, input: &[i16], output: &mut Vec<i16>) {
        self.frames.extend(
            input
                .as_chunks::<2>()
                .0
                .iter()
                .map(|pair| [f32::from(pair[0]), f32::from(pair[1])]),
        );
        // A frame ahead of the one interpolated from is needed, and one beyond it.
        while self.position as usize + 2 < self.frames.len() {
            let index = self.position as usize;
            let t = (self.position - index as f64) as f32;
            for channel in 0..2 {
                let (p0, p1, p2, p3) = (
                    self.frames[index - 1][channel],
                    self.frames[index][channel],
                    self.frames[index + 1][channel],
                    self.frames[index + 2][channel],
                );
                let value = 0.5
                    * (2.0 * p1
                        + (p2 - p0) * t
                        + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t * t
                        + (3.0 * p1 - p0 - 3.0 * p2 + p3) * t * t * t);
                output.push(value.round().clamp(-32_768.0, 32_767.0) as i16);
            }
            self.position += self.step;
        }
        let passed = (self.position as usize).saturating_sub(1);
        self.frames.drain(..passed);
        self.position -= passed as f64;
    }
}

/// Puts blocks of the player's mix on the recording's clock, as the interleaved 16-bit little-endian stereo samples ffmpeg reads at 48 kHz.
///
/// A block is put right when the sound is more than [`TOLERANCE`] from the clock, by silence before it or by leaving out its early part, which are hard cuts and may click; a device whose clock differs from the recording's by 50 parts in a million needs one about every 400 s. A crossfade, or resampling by the drift, would hide them, and is a follow-up. After every block the sound is within [`TOLERANCE`] of the clock at the time the block was rendered.
#[derive(Default)]
pub struct Aligner {
    /// The frames written, which are the frames of the file.
    written: u64,
    /// The rate being resampled from, and the resampler.
    resampler: Option<(u32, Resampler)>,
    padded: u64,
    trimmed: u64,
}

fn frames_in(duration: Duration) -> i64 {
    (duration.as_nanos() * u128::from(SAMPLE_RATE) / 1_000_000_000) as i64
}

fn write_samples(samples: &[i16], bytes: &mut Vec<u8>) {
    bytes.extend(samples.iter().flat_map(|sample| sample.to_le_bytes()));
}

impl Aligner {
    fn silence(&mut self, frames: u64, bytes: &mut Vec<u8>) {
        bytes.resize(bytes.len() + frames as usize * CHANNELS * 2, 0);
        self.written += frames;
        self.padded += frames;
    }

    /// `block` as 48 kHz stereo.
    fn stereo(&mut self, block: &Block) -> Vec<i16> {
        let stereo: Vec<i16> = match block.layout {
            Layout::Stereo => block.samples.clone(),
            Layout::Mono => block
                .samples
                .iter()
                .flat_map(|&sample| [sample, sample])
                .collect(),
        };
        if block.rate == 0 || block.rate == SAMPLE_RATE {
            self.resampler = None;
            return stereo;
        }
        let resampler = match &mut self.resampler {
            Some((rate, resampler)) if *rate == block.rate => resampler,
            other => {
                &mut other
                    .insert((block.rate, Resampler::new(block.rate, SAMPLE_RATE)))
                    .1
            }
        };
        let mut resampled = Vec::with_capacity(stereo.len() * 2);
        resampler.push(&stereo, &mut resampled);
        resampled
    }

    /// Append `block` to `bytes`, preceded by the silence that puts it where its time says, or without the start of it that is early.
    pub fn block(&mut self, block: &Block, bytes: &mut Vec<u8>) {
        let samples = self.stereo(block);
        let frames = (samples.len() / CHANNELS) as i64;
        let started = block.at.saturating_sub(Duration::from_nanos(
            (frames as u128 * 1_000_000_000 / u128::from(SAMPLE_RATE)) as u64,
        ));
        let gap = frames_in(started) - self.written as i64;
        let mut skipped = 0;
        if gap > TOLERANCE {
            self.silence(gap as u64, bytes);
        } else if gap < -TOLERANCE {
            skipped = (-gap).min(frames);
            self.trimmed += skipped as u64;
        }
        write_samples(&samples[skipped as usize * CHANNELS..], bytes);
        self.written += (frames - skipped) as u64;
    }

    /// Fill the time up to `now` with silence, if nothing has arrived for so long that the audio has fallen well behind it.
    pub fn idle(&mut self, now: Duration, bytes: &mut Vec<u8>) {
        let behind = frames_in(now) - self.written as i64;
        if behind > IDLE {
            self.silence(behind as u64, bytes);
        }
    }

    /// Fill with silence up to `frames` in all, if fewer have been written: the end of the recording, which has the length of its video.
    pub fn finish(&mut self, frames: u64, bytes: &mut Vec<u8>) {
        if frames > self.written {
            self.silence(frames - self.written, bytes);
        }
    }

    /// The frames written.
    pub fn written(&self) -> u64 {
        self.written
    }
}

/// What the audio writer did, for the report.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Totals {
    /// Frames of 48 kHz stereo written.
    pub frames: u64,
    /// Of those, frames of silence put in where no sound was rendered.
    pub padded: u64,
    /// Frames of a block that came early and were left out.
    pub trimmed: u64,
    /// Why writing stopped, if it did before the recording ended.
    pub error: Option<String>,
}

/// Write the blocks that arrive to `pipe`, timed by the clock that started at `start`, until the recording ends, and then silence up to the length of its video.
pub fn write(
    mut pipe: impl Write + Send + 'static,
    blocks: Receiver<Block>,
    origin: Origin,
    clock: FrameClock,
    ending: Arc<Ending>,
    stop: Stop,
) -> std::io::Result<JoinHandle<Totals>> {
    system::spawn("tomori-record-audio", move || {
        let mut aligner = Aligner::default();
        // The sound not yet written: the last of it is held back, so that a recording that ends can be cut to the length of its video, as the sound may be a little ahead of the clock.
        let mut bytes = Vec::new();
        let mut sent = 0;
        let mut error = None;
        let held = frames_in(clock.interval() + HELD) as usize * CHANNELS * 2;
        loop {
            let ended = ending.at();
            match blocks.recv_timeout(BLOCK) {
                Ok(block) => aligner.block(&block, &mut bytes),
                Err(RecvTimeoutError::Timeout) => aligner.idle(origin.elapsed(), &mut bytes),
                Err(RecvTimeoutError::Disconnected) => {
                    thread::sleep(BLOCK);
                    aligner.idle(origin.elapsed(), &mut bytes);
                }
            }
            let writable = if let Some(end) = ended {
                // What was rendered before the end, then silence to the length of the video, and no more.
                while let Ok(block) = blocks.try_recv() {
                    aligner.block(&block, &mut bytes);
                }
                let video = frames_in(clock.timestamp(clock.frames_in(end))) as usize;
                aligner.finish(video as u64, &mut bytes);
                let allowed = (video * CHANNELS * 2).saturating_sub(sent);
                bytes.truncate(allowed);
                bytes.len()
            } else {
                bytes.len().saturating_sub(held)
            };
            if let Err(failed) = pipe.write_all(&bytes[..writable]) {
                error = Some(format!("cannot write the sound to ffmpeg: {failed}"));
                stop.request("the encoder stopped");
                break;
            }
            sent += writable;
            bytes.drain(..writable);
            if ended.is_some() {
                break;
            }
        }
        let _ = pipe.flush();
        Totals {
            frames: (sent / (CHANNELS * 2)) as u64,
            padded: aligner.padded,
            trimmed: aligner.trimmed,
            error,
        }
    })
}

/// Renders the player at 48 kHz in blocks of 10 ms, paced by the clock, until dropped, when the spectator does not play to a speaker.
struct Driver {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Driver {
    fn start(player: Shared) -> std::io::Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let thread = system::spawn("tomori-record-player", move || {
            let mut samples = vec![0; frames_in(BLOCK) as usize * CHANNELS];
            let mut next = Instant::now();
            while !stopping.load(Ordering::Relaxed) {
                player
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .render(SAMPLE_RATE, Layout::Stereo, &mut samples);
                next += BLOCK;
                thread::sleep(next.saturating_duration_since(Instant::now()));
            }
        })?;
        Ok(Self {
            stop,
            thread: Some(thread),
        })
    }
}

impl Drop for Driver {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.thread.take().map(JoinHandle::join);
    }
}

/// The spectator's playback while it records: the speaker it plays to, if it plays to one, with the recording tapped onto the player it renders; or, with none, or if the speaker cannot be opened, the recording alone, which renders the player itself.
pub struct Tee {
    speaker: Option<Box<dyn Playback>>,
    tap: AudioTap,
}

impl Tee {
    pub fn new(speaker: Option<Box<dyn Playback>>, tap: AudioTap) -> Self {
        Self { speaker, tap }
    }
}

impl Playback for Tee {
    fn open(&self, player: Shared, failed: Failed, route: Route) -> Result<Output, String> {
        player
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .tap(Some(self.tap.tap()));
        if let Some(speaker) = &self.speaker {
            match speaker.open(player.clone(), failed, route) {
                Ok(output) => return Ok(output),
                Err(error) => {
                    log::warn!("Recording: {error}; the sound is recorded without playing it");
                }
            }
        }
        // The cues are synthesized here, outside the callback, for the rate the recording renders at; a speaker prepares its own.
        player
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .prepare(SAMPLE_RATE);
        Driver::start(player).map(Output::new).map_err(|error| {
            format!("Cannot start the thread that renders the recording's sound: {error}")
        })
    }

    fn reopens_on_focus(&self) -> bool {
        self.speaker
            .as_ref()
            .is_some_and(|speaker| speaker.reopens_on_focus())
    }
}

#[cfg(test)]
mod tests;
