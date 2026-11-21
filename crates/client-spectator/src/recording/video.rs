// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The recording's video writer: frames the render loop has drawn arrive with the time they were drawn at, and a thread of its own writes one to the encoder for each slot of the frame clock, repeating the frame before when none newer was drawn, so that the render loop never waits for the encoder and the file never loses time. See [`super::clock`].
//!
//! The thread writes a slot once it is two frames old, so that a frame drawn at the slot's time has been read back from the GPU and is there to choose, rather than the slot repeating the one before. This delays what the encoder is given, not what the file shows: frames are placed by the time they were drawn at.
use super::{
    Origin, Stop,
    clock::{Counts, FrameClock, Pacer},
    system,
};
use std::{
    io::Write,
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

/// How many bytes of frames the render loop may have waiting for the writer, which an encoder that cannot keep up lets fill; at least [`MINIMUM_QUEUED`] frames, and at most [`MAXIMUM_QUEUED`], each a whole frame of RGBA.
const QUEUED_BYTES: usize = 128 * 1024 * 1024;
const MINIMUM_QUEUED: usize = 4;
const MAXIMUM_QUEUED: usize = 16;
/// How many frames behind the clock a slot is written, for the frame drawn at its time to arrive.
const LEAD: u32 = 2;
/// The longest the writer sleeps without looking for a frame or the end.
const LOOK: Duration = Duration::from_millis(20);
/// How far behind the clock the writer may fall before it says so.
const BEHIND: Duration = Duration::from_secs(1);

/// A frame, shared between the slots it fills.
pub type Frame = Arc<Vec<u8>>;

/// When the recording ends, said once by whoever finishes it, to the writers of the video and the sound.
#[derive(Default)]
pub struct Ending(Mutex<Option<Duration>>);

impl Ending {
    /// The recording ends at `at` on its clock.
    pub fn end(&self, at: Duration) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = Some(at);
    }

    /// When it ends, once that has been said.
    pub fn at(&self) -> Option<Duration> {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Hands frames to the video writer from the render loop, without waiting for it.
#[derive(Clone)]
pub struct FrameSender {
    sender: SyncSender<(Duration, Frame)>,
    origin: Origin,
    lost: Arc<AtomicU64>,
}

impl FrameSender {
    /// A sender for a recording that began at `origin`, and the frames it sends, at most as many as 128 MiB holds of frames of `frame_bytes`, and not fewer than four.
    pub fn new(origin: Origin, frame_bytes: usize) -> (Self, Receiver<(Duration, Frame)>) {
        let queued = (QUEUED_BYTES / frame_bytes.max(1)).clamp(MINIMUM_QUEUED, MAXIMUM_QUEUED);
        let (sender, frames) = mpsc::sync_channel(queued);
        let sender = Self {
            sender,
            origin,
            lost: Arc::default(),
        };
        (sender, frames)
    }

    /// Hand over `frame`, drawn at `drawn`. If the writer has fallen so far behind that [`QUEUED`] frames wait, the frame is dropped, and counted, rather than the render loop waiting; returns whether it was sent.
    pub fn send(&self, drawn: Instant, frame: Vec<u8>) -> bool {
        let at = self.origin.since(drawn);
        match self.sender.try_send((at, Arc::new(frame))) {
            Ok(()) => true,
            Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => {
                self.lost.fetch_add(1, Ordering::Relaxed);
                false
            }
        }
    }

    /// Count a frame that was drawn and not sent, because it could not be read back.
    pub fn lose(&self) {
        self.lost.fetch_add(1, Ordering::Relaxed);
    }

    /// How many frames were drawn and not used by the file: dropped because the writer was too far behind, or never read back.
    pub fn lost(&self) -> u64 {
        self.lost.load(Ordering::Relaxed)
    }
}

/// What the video writer did, for the report.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Totals {
    pub counts: Counts,
    /// The most the writer was behind the clock, once it began writing.
    pub behind: Duration,
    /// Why writing stopped, if it did before the recording ended.
    pub error: Option<String>,
}

/// Write the frames that arrive to `pipe`, one for each slot of `clock` that has come, until the recording ends at the time `ending` says, and then every slot of the recording, so that the file is as long as its time. `frame_bytes` is the size of a frame.
pub fn write(
    mut pipe: impl Write + Send + 'static,
    frames: Receiver<(Duration, Frame)>,
    origin: Origin,
    clock: FrameClock,
    ending: Arc<Ending>,
    stop: Stop,
    frame_bytes: usize,
) -> std::io::Result<JoinHandle<Totals>> {
    system::spawn("tomori-record-video", move || {
        let mut pacer = Pacer::new(clock);
        let mut totals = Totals::default();
        let lead = clock.interval() * LEAD;
        loop {
            let ended = ending.at();
            while let Ok((at, frame)) = frames.try_recv() {
                pacer.draw(at, frame);
            }
            // A recording that ended before its first frame was drawn is black for its length.
            if ended.is_some() && !pacer.ready() {
                pacer.draw(Duration::ZERO, Arc::new(vec![0; frame_bytes]));
            }
            let elapsed = origin.elapsed();
            let slots = match ended {
                Some(end) => clock.frames_in(end),
                None => elapsed.checked_sub(lead).map_or(0, |time| clock.due(time)),
            };
            // How far the next slot to write is behind its time, once there is a frame to write.
            let behind = if pacer.ready() {
                elapsed
                    .saturating_sub(clock.timestamp(pacer.next()))
                    .saturating_sub(lead)
            } else {
                Duration::ZERO
            };
            // Said each time it is a second further behind than it has been.
            if behind > totals.behind + BEHIND {
                log::warn!(
                    "Recording: the encoder is {:.1} s behind; frames are repeated to keep the video's time",
                    behind.as_secs_f32()
                );
            }
            totals.behind = totals.behind.max(behind);
            if let Err(error) = pacer.write(slots, |frame| pipe.write_all(frame)) {
                totals.error = Some(format!("cannot write the video to ffmpeg: {error}"));
                stop.request("the encoder stopped");
                break;
            }
            if ended.is_some() && pacer.next() >= slots {
                break;
            }
            let next = clock.timestamp(pacer.next()) + lead;
            let wait = next
                .saturating_sub(elapsed)
                .clamp(Duration::from_millis(1), LOOK);
            match frames.recv_timeout(wait) {
                Ok((at, frame)) => pacer.draw(at, frame),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => thread::sleep(wait),
            }
        }
        let _ = pipe.flush();
        totals.counts = pacer.counts();
        totals
    })
}

#[cfg(test)]
mod tests;
