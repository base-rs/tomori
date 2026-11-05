// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The platform microphone, opened for live audio or push-to-talk, behind a trait that each client's devices implement. See `design/live-audio.md`.
//!
//! Capture hands mono samples to a callback on its own thread; [`crate::connection::Connection`] passes them to the client's [`crate::Microphone`], which frames and sends them. `tomori-scene`'s `capture` module has helpers that keep a device adapter thin.

/// Receives captured mono samples in the range -1 to 1, on the capture thread.
pub type Samples = Box<dyn FnMut(&[f32]) + Send>;

/// Told, from any thread, why capture stopped early.
pub type Failed = Box<dyn Fn(String) + Send + Sync>;

/// Whether the connection on which live capture started is still usable.
pub type Active = Box<dyn Fn() -> bool + Send>;

/// Capture in progress, stopped when dropped. Samples already delivered are not lost.
pub struct Recording {
    /// The rate the microphone captures at, in samples a second.
    pub sample_rate: u32,
    _stream: Box<dyn std::any::Any>,
}

impl Recording {
    /// Capture at `sample_rate` by `stream`, which is kept until the recording is dropped and stops capturing when it is.
    pub fn new(sample_rate: u32, stream: impl std::any::Any) -> Self {
        Self {
            sample_rate,
            _stream: Box::new(stream),
        }
    }
}

/// A source of microphone audio.
pub trait Capture {
    /// Whether the app may record, asking the participant if it has not yet. A request is answered later, so `Ok(false)` means ask again after they respond.
    fn permit(&mut self) -> Result<bool, String> {
        Ok(true)
    }

    /// Ask for permission ahead of the first recording, without using up the
    /// request [`Self::permit`] makes, so that a dismissed request is asked
    /// again when the participant first uses the microphone.
    fn request(&mut self) -> Result<bool, String> {
        Ok(true)
    }

    /// Start capturing at a rate between 8 and 48 kHz, which the protocol carries.
    fn open(&mut self, samples: Samples, failed: Failed) -> Result<Recording, String>;

    /// Capture for an open microphone, stopping when its connection is lost.
    /// Platforms must opt in with a capture implementation that can stop on
    /// the capture thread, even if the application thread is stalled.
    fn open_live(
        &mut self,
        _samples: Samples,
        _failed: Failed,
        _active: Active,
    ) -> Result<Recording, String> {
        Err("Live microphone is not available on this platform.".into())
    }

    /// The audio session the open live microphone captures in, where the platform allocates one, so that the speaker can play in the same session while it is open. See `design/spatial-audio.md`.
    fn live_session(&self) -> Option<i32> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A capture that opens only for push-to-talk, and asks nothing.
    struct PushToTalk;

    impl Capture for PushToTalk {
        fn open(&mut self, _samples: Samples, _failed: Failed) -> Result<Recording, String> {
            Ok(Recording::new(16_000, ()))
        }
    }

    #[test]
    fn a_capture_is_permitted_unless_it_says_otherwise_and_refuses_live_audio_unless_it_opts_in() {
        let mut capture = PushToTalk;
        assert_eq!(capture.permit(), Ok(true));
        assert_eq!(capture.request(), Ok(true));
        assert_eq!(capture.live_session(), None);
        let mut samples: Samples = Box::new(|_| {});
        let failed: Failed = Box::new(|_| {});
        // What a capture that opened would call.
        samples(&[]);
        failed(String::new());
        let recording = capture.open(samples, failed).unwrap();
        assert_eq!(recording.sample_rate, 16_000);
        // What a capture that opened would call.
        let samples = |_: &[f32]| {};
        let failed = |_: String| {};
        let active = || true;
        samples(&[]);
        failed(String::new());
        assert!(active());
        let error = capture
            .open_live(Box::new(samples), Box::new(failed), Box::new(active))
            .map(drop)
            .unwrap_err();
        assert!(error.contains("not available"));
    }
}
