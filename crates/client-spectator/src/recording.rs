// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Recording what the spectator shows and plays to a video file with sound, `--record FILE`. See `design/spectator.md`.
//!
//! The spectator renders to a texture of the recording's size, which is read back and handed to the video writer, with the time it was drawn at on the recording's clock; the sound its player renders is copied, as the speaker is given it, to the audio writer, which times it by the same clock. An ffmpeg child process encodes both and writes the file. Nothing here waits for the encoder: a frame the encoder is too slow for is dropped before the render loop would wait, and the file keeps its time by repeating the frame before ([`clock`]).
//!
//! * [`options`]: what is asked, and what is refused.
//! * [`clock`]: the constant frame rate and how drawn frames fill its slots.
//! * [`ffmpeg`]: finding ffmpeg and an encoder, and its command line.
//! * [`audio`]: the tap on the player, the 48 kHz stereo stream timed by the clock, and the playback that feeds both the speaker and the recording.
//! * [`video`]: the thread that writes the video's slots to ffmpeg.
//! * [`capture`]: the render target and its GPU readback.
//! * [`session`]: starting ffmpeg and its threads, and finishing the file.
use std::{
    sync::{
        Arc, Mutex, OnceLock, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

pub mod audio;
pub mod capture;
pub mod clock;
pub mod ffmpeg;
pub mod options;
mod process;
pub mod session;
pub mod signals;
mod system;
pub mod video;

pub use options::{Codec, Container, RecordOptions};
pub use session::{Report, Session};

/// The texture format the recording renders in, whose bytes are the frames ffmpeg reads: RGBA, sRGB-encoded.
pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
/// The sound's sample rate in the file.
pub const SAMPLE_RATE: u32 = 48_000;
/// The sound's channels in the file: stereo, as the spectator plays.
pub const CHANNELS: usize = 2;

/// Why and whether a recording is to end: asked by whatever notices, the window closing, a signal, the server going away, or the encoder failing, and looked at by what runs the recording, which finishes the file. The first reason stays.
#[derive(Clone, Debug, Default)]
pub struct Stop(Arc<StopState>);

#[derive(Debug, Default)]
struct StopState {
    requested: AtomicBool,
    reason: Mutex<Option<String>>,
    /// Whether signals that end the program also ask for this: only a running recording's.
    signals: bool,
}

impl Stop {
    /// A stop that the signals that end a program ask for too, once [`signals::catch`] has them noted.
    pub fn on_signals() -> Self {
        Self(Arc::new(StopState {
            signals: true,
            ..StopState::default()
        }))
    }

    /// Ask for the recording to end, for `reason`, unless it was asked already.
    pub fn request(&self, reason: impl Into<String>) {
        let mut kept = self.0.reason.lock().unwrap_or_else(PoisonError::into_inner);
        if kept.is_none() {
            let reason = reason.into();
            log::info!("Recording: stopping, {reason}");
            *kept = Some(reason);
        }
        self.0.requested.store(true, Ordering::Release);
    }

    /// Whether the recording is to end.
    pub fn requested(&self) -> bool {
        if self.0.signals
            && let Some(signal) = signals::received()
        {
            self.request(format!("received {signal}"));
        }
        self.0.requested.load(Ordering::Acquire)
    }

    /// Why it is to end, once it is.
    pub fn reason(&self) -> Option<String> {
        self.0
            .reason
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// When a recording begins, which is when its first frame is drawn, and not when the encoder starts, which is before the window and the GPU are ready: its video, its sound and its report are timed from then, so that the file does not begin with the seconds the spectator took to start. Shared by whatever draws, copies the sound, writes and finishes the recording.
#[derive(Clone, Debug, Default)]
pub struct Origin(Arc<OnceLock<Instant>>);

impl Origin {
    /// An origin that began at `at`.
    pub fn at(at: Instant) -> Self {
        let origin = Self::default();
        origin.begin(at);
        origin
    }

    /// Begin the recording at `at`, unless it began already.
    pub fn begin(&self, at: Instant) {
        let _ = self.0.set(at);
    }

    /// Whether the recording has begun.
    pub fn begun(&self) -> bool {
        self.0.get().is_some()
    }

    /// How long the recording has gone on, or nothing before it begins.
    pub fn elapsed(&self) -> Duration {
        self.0.get().map_or(Duration::ZERO, Instant::elapsed)
    }

    /// When `at` was in the recording, or nothing before it began.
    pub fn since(&self, at: Instant) -> Duration {
        self.0
            .get()
            .map_or(Duration::ZERO, |start| at.saturating_duration_since(*start))
    }
}

/// What the render loop needs of a running recording: where to send frames, when it began, and whether to go on.
#[derive(Clone)]
pub struct Recording {
    pub size: (u32, u32),
    pub fps: u32,
    pub origin: Origin,
    pub frames: video::FrameSender,
    pub stop: Stop,
}

impl Recording {
    /// End the recording if the server has gone: the spectator has nothing more to show it of the space, and would otherwise go on recording its wait for another.
    pub fn watch(&self, feed: &crate::spectator_feed::Feed) {
        if feed.connection_lost() {
            self.stop.request("the server disconnected");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_reason_to_stop_stays() {
        let stop = Stop::default();
        assert!(!stop.requested());
        assert_eq!(stop.reason(), None);
        stop.request("the window closed");
        stop.clone().request("the server disconnected");
        assert!(stop.requested());
        assert_eq!(stop.reason().as_deref(), Some("the window closed"));
    }
}
