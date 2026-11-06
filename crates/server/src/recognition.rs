// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! A [`Transcriber`] backed by the speech crate's streaming [`Recognition`]. See `design/speech.md`.
use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

use tomori_protocol::AudioFrame;
use tomori_speech::stt::{RATE, Transcript};
use tomori_speech::{Error, Recognition, Stopper, StreamingRecognizer, TranscriptEvent, Utterance};

use crate::transcriber::{Transcriber, TranscriptSink, UtteranceId};

/// Reported when recognition has stopped: a panic disabled it, or the server is shutting down. Trying again does not help, so the message does not suggest it.
const STOPPED: &str =
    "Speech recognition is unavailable until the server restarts, so nothing was sent.";

/// Streaming speech recognition for push-to-talk utterances.
///
/// One recognizer thread serves every utterance on the server. Partial transcripts are reported while the participant is speaking; a recognizer error, including a missing model or recognition falling too far behind, fails the utterance with a visible message. Once a panic has disabled recognition, every utterance fails saying so, until the server restarts.
pub struct RecognitionTranscriber {
    recognition: Mutex<Recognition>,
    utterances: Mutex<HashMap<UtteranceId, Utterance>>,
}

impl RecognitionTranscriber {
    /// Transcribe with a streaming `recognizer`, which decodes the audio once, as it arrives.
    pub fn new(recognizer: impl StreamingRecognizer + 'static) -> Self {
        Self {
            recognition: Mutex::new(Recognition::spawn(recognizer)),
            utterances: Mutex::default(),
        }
    }

    /// A handle that stops the recognition thread when the server shuts down; see `design/server.md`.
    pub fn stopper(&self) -> Stopper {
        self.recognition
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .stopper()
    }

    fn utterances(&self) -> std::sync::MutexGuard<'_, HashMap<UtteranceId, Utterance>> {
        self.utterances
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

impl Transcriber for RecognitionTranscriber {
    fn begin(&self, utterance: UtteranceId, sink: TranscriptSink) {
        let mut reporter = Reporter {
            sink,
            reported: false,
        };
        // The sample rate is adapted to each frame as it arrives.
        let handle = self
            .recognition
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .begin(utterance.0, RATE, move |event| reporter.report(event));
        self.utterances().insert(utterance, handle);
    }

    fn push(&self, utterance: UtteranceId, frame: &AudioFrame) {
        if let Some(handle) = self.utterances().get_mut(&utterance) {
            handle.push_frame(frame);
        }
    }

    fn end(&self, utterance: UtteranceId) {
        let handle = self.utterances().remove(&utterance);
        if let Some(handle) = handle {
            handle.end();
        }
    }

    /// The hub cancels from inside a failure report, which can arrive while `begin` holds `recognition`, so this must not take that lock.
    fn cancel(&self, utterance: UtteranceId) {
        let handle = self.utterances().remove(&utterance);
        if let Some(handle) = handle {
            handle.cancel();
        }
    }
}

/// Forwards one utterance's events to its sink. If recognition drops it without a final result, the utterance fails; after a cancellation the server ignores that report.
struct Reporter {
    sink: TranscriptSink,
    reported: bool,
}

impl Reporter {
    fn report(&mut self, event: TranscriptEvent) {
        match event {
            TranscriptEvent::Partial(transcript) => self.sink.partial(&text(&transcript)),
            TranscriptEvent::Progress(_) => {
                unreachable!("the streaming recognizer reports no progress")
            }
            TranscriptEvent::Final(text) => {
                self.reported = true;
                self.sink.finish(&text);
            }
            TranscriptEvent::Failed(Error::Stopped(_)) => {
                self.reported = true;
                self.sink.fail(STOPPED);
            }
            TranscriptEvent::Failed(error) => {
                self.reported = true;
                self.sink.fail(&format!(
                    "Transcription failed, so nothing was sent: {error}."
                ));
            }
        }
    }
}

impl Drop for Reporter {
    fn drop(&mut self) {
        if !self.reported {
            self.sink.fail(STOPPED);
        }
    }
}

/// The committed and tentative words as one line.
fn text(transcript: &Transcript) -> String {
    [transcript.committed.trim(), transcript.tentative.trim()]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A recognizer without its model.
    struct Missing;

    impl StreamingRecognizer for Missing {
        fn open(&mut self) -> Result<Box<dyn tomori_speech::RecognitionStream>, Error> {
            Err(Error::Unavailable("the model is missing".into()))
        }
    }

    #[test]
    fn audio_for_an_utterance_it_did_not_begin_is_ignored() {
        assert!(Missing.open().is_err());
        let transcriber = RecognitionTranscriber::new(Missing);
        let frame = AudioFrame {
            gate: tomori_protocol::Gate::Agent,
            source: 0,
            sequence: 0,
            timestamp: 0,
            sample_rate: RATE,
            samples: vec![0; 160],
        };
        transcriber.push(UtteranceId(9), &frame);
        transcriber.end(UtteranceId(9));
        transcriber.cancel(UtteranceId(9));
        let stopper = transcriber.stopper();
        stopper.stop();
        assert!(stopper.wait(std::time::Instant::now() + std::time::Duration::from_secs(5)));
    }
}
