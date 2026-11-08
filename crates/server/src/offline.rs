// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! A [`Transcriber`] that buffers each utterance and decodes it whole, once, with the offline recognizer. See `design/speech.md`.
//!
//! The offline recognizer is more accurate than the streaming one, but it cannot stream: it decodes a complete clip, so the transcript arrives a couple of seconds after the participant lets go, instead of almost at once. An utterance longer than its 30 second window is split at its pauses and decoded in parts, up to [`tomori_speech::stt::MAX_UTTERANCE`]; each part is decoded as soon as the audio decides where it ends, while the participant is still speaking, and the words of the parts decoded so far are shown to them as a partial transcript, so that only the last part, or at most two, is left once they let go.
use std::{
    collections::HashMap,
    sync::{Mutex, PoisonError},
};

use tomori_protocol::{AudioFrame, TranscriptPart, TranscriptStage};
use tomori_speech::stt::{
    OfflineRecognition, OfflineRecognizer, OfflineUtterance, RATE, VoiceActivity,
};
use tomori_speech::{Error, Progress, Stopper};

use crate::transcriber::{Transcriber, TranscriptSink, UtteranceId};

/// Reported when recognition has stopped: a panic disabled it, or the server is shutting down. Trying again does not help, so the message does not suggest it.
const STOPPED: &str =
    "Speech recognition is unavailable until the server restarts, so nothing was sent.";

/// Offline speech recognition for push-to-talk utterances.
///
/// Each utterance's audio is buffered and decoded whole, a part at a time for a long one, the parts whose ends are decided while the participant is still speaking, and the rest once they stop. A recognizer error fails the utterance with a visible message; once a panic has disabled recognition, every utterance fails saying so, until the server restarts.
pub struct OfflineTranscriber {
    offline: Mutex<OfflineRecognition>,
    utterances: Mutex<HashMap<UtteranceId, OfflineUtterance>>,
}

impl OfflineTranscriber {
    /// Buffer and decode each utterance with an offline `recognizer`, splitting one too long for it at the pauses `voice` hears, or at its quietest points without one.
    pub fn new(
        recognizer: impl OfflineRecognizer + 'static,
        voice: Option<Box<dyn VoiceActivity>>,
    ) -> Self {
        let offline = match voice {
            Some(voice) => OfflineRecognition::with_voice_activity(recognizer, voice),
            None => OfflineRecognition::spawn(recognizer),
        };
        Self {
            offline: Mutex::new(offline),
            utterances: Mutex::default(),
        }
    }

    /// A handle that stops the decode thread when the server shuts down; see `design/server.md`.
    pub fn stopper(&self) -> Stopper {
        self.offline
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .stopper()
    }

    fn utterances(&self) -> std::sync::MutexGuard<'_, HashMap<UtteranceId, OfflineUtterance>> {
        self.utterances
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

impl Transcriber for OfflineTranscriber {
    fn begin(&self, utterance: UtteranceId, sink: TranscriptSink) {
        let handle = self
            .offline
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .begin(RATE, move |event| report(&sink, event));
        self.utterances().insert(utterance, handle);
    }

    /// Append the utterance's audio; it is buffered on the decode thread, which decodes each part as soon as the audio decides where it ends.
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

    /// The hub cancels from inside a failure report, which can arrive while `begin` holds `offline`, so this must not take that lock.
    fn cancel(&self, utterance: UtteranceId) {
        let handle = self.utterances().remove(&utterance);
        if let Some(handle) = handle {
            handle.cancel();
        }
    }

    /// The whole clip is buffered, up to [`tomori_speech::stt::MAX_UTTERANCE`].
    fn utterance_limit(&self) -> Option<std::time::Duration> {
        Some(std::time::Duration::from_secs(
            (tomori_speech::stt::MAX_UTTERANCE / RATE as usize) as u64,
        ))
    }
}

/// Tell `sink` of an utterance's `event`.
fn report(sink: &TranscriptSink, event: tomori_speech::TranscriptEvent) {
    match event {
        // The words of the parts decoded so far, all of them committed.
        tomori_speech::TranscriptEvent::Partial(transcript) => {
            sink.partial(transcript.committed.trim());
        }
        tomori_speech::TranscriptEvent::Progress(Progress::Waiting) => {
            sink.progress(TranscriptStage::Waiting, None);
        }
        tomori_speech::TranscriptEvent::Progress(Progress::Transcribing { part, parts }) => {
            // A part is worth saying only of several; the counts are bounded by the 5 minute limit.
            let part = (parts > 1).then(|| TranscriptPart {
                number: u32::try_from(part).unwrap_or(u32::MAX),
                of: u32::try_from(parts).unwrap_or(u32::MAX),
            });
            sink.progress(TranscriptStage::Transcribing, part);
        }
        tomori_speech::TranscriptEvent::Final(text) => sink.finish(&text),
        tomori_speech::TranscriptEvent::Failed(Error::Stopped(_)) => sink.fail(STOPPED),
        tomori_speech::TranscriptEvent::Failed(error @ Error::TooLong(_)) => sink.fail(&format!(
            "Nothing was sent: {error}. Say it in shorter messages."
        )),
        tomori_speech::TranscriptEvent::Failed(error) => sink.fail(&format!(
            "Transcription failed, so nothing was sent: {error}."
        )),
    }
}

/// An [`OfflineRecognizer`] for the in-process Whisper model: it decodes each buffered clip, or segment of a long one, whole, once.
pub struct Whisper {
    inner: tomori_speech::stt::WhisperRecognizer,
}

impl Whisper {
    /// Load the Whisper model from `config`.
    pub fn new(config: &tomori_speech::stt::WhisperConfig) -> Result<Self, Error> {
        Ok(Self {
            inner: tomori_speech::stt::WhisperRecognizer::new(config)?,
        })
    }
}

impl OfflineRecognizer for Whisper {
    fn transcribe(&mut self, samples: &[i16]) -> Result<String, Error> {
        self.inner.transcribe(samples)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A recognizer that hears nothing.
    struct Deaf;

    impl OfflineRecognizer for Deaf {
        fn transcribe(&mut self, _: &[i16]) -> Result<String, Error> {
            Ok(String::new())
        }
    }

    #[test]
    fn each_outcome_is_told_to_the_utterance() {
        let sink = TranscriptSink::nowhere(UtteranceId(1));
        for event in [
            tomori_speech::TranscriptEvent::Partial(tomori_speech::stt::Transcript {
                committed: "Deploy".into(),
                tentative: String::new(),
            }),
            tomori_speech::TranscriptEvent::Progress(Progress::Waiting),
            tomori_speech::TranscriptEvent::Progress(Progress::Transcribing { part: 1, parts: 1 }),
            tomori_speech::TranscriptEvent::Progress(Progress::Transcribing { part: 2, parts: 3 }),
            tomori_speech::TranscriptEvent::Final("Deploy it.".into()),
            tomori_speech::TranscriptEvent::Failed(Error::Stopped("gone".into())),
            tomori_speech::TranscriptEvent::Failed(Error::TooLong(600)),
            tomori_speech::TranscriptEvent::Failed(Error::Engine("bad audio".into())),
        ] {
            report(&sink, event);
        }
    }

    #[test]
    fn audio_for_an_utterance_it_did_not_begin_is_ignored() {
        let transcriber = OfflineTranscriber::new(Deaf, None);
        assert_eq!(Deaf.transcribe(&[]).unwrap(), "");
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

    #[test]
    fn whisper_transcribes_a_clip() {
        use tomori_speech::test_models::{self, WHISPER_MODEL};
        let config = tomori_speech::stt::WhisperConfig::named(&test_models::root(), WHISPER_MODEL);
        test_models::with(
            "Whisper",
            "scripts/fetch-asr-model.sh whisper tiny.en",
            &[&config.model],
            || {
                let mut whisper = Whisper::new(&config).unwrap();
                // Whisper names silence, as "[ Silence ]", rather than saying nothing.
                assert!(whisper.transcribe(&[0; 16_000]).is_ok());
            },
        );
    }
}
