// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Silero voice activity detection in-process through sherpa-onnx, which finds the pauses at which the offline recognizer splits a long utterance. See [`super::segment`].

use std::{
    ops::Range,
    path::{Path, PathBuf},
};

use sherpa_onnx::{SileroVadModelConfig, VadModelConfig, VoiceActivityDetector};

use super::{RATE, VoiceActivity, segment::MAX_SEGMENT};
use crate::Error;

/// The samples Silero decides on at once, at 16 kHz.
const WINDOW: usize = 512;

/// The most audio the detector hears at once: one segment, which is more than the search for a pause and the audio around it.
const LONGEST: usize = MAX_SEGMENT;

/// Where to find the Silero voice activity model and how to run it.
#[derive(Clone, Debug)]
pub struct SileroVadConfig {
    /// `silero_vad.onnx`.
    pub model: PathBuf,
    /// The probability above which a window is speech.
    pub threshold: f32,
    /// The shortest silence, in seconds, that ends a stretch of speech. Pauses shorter than this are heard as speech.
    pub min_silence: f32,
    /// The shortest stretch of speech, in seconds, that is reported.
    pub min_speech: f32,
}

impl SileroVadConfig {
    /// The model fetched by `scripts/fetch-asr-model.sh whisper` under the repository `root`: `.tools/asr/silero_vad.onnx`.
    pub fn from_root(root: &Path) -> Self {
        Self {
            model: root.join(".tools/asr/silero_vad.onnx"),
            threshold: 0.5,
            min_silence: 0.1,
            min_speech: 0.1,
        }
    }

    fn sherpa(&self) -> Result<VadModelConfig, Error> {
        if !self.model.exists() {
            return Err(Error::Unavailable(format!(
                "the Silero voice activity model is not installed at {}; run bash scripts/fetch-asr-model.sh whisper",
                self.model.display()
            )));
        }
        Ok(VadModelConfig {
            silero_vad: SileroVadModelConfig {
                model: Some(self.model.to_string_lossy().into_owned()),
                threshold: self.threshold,
                min_silence_duration: self.min_silence,
                min_speech_duration: self.min_speech,
                window_size: WINDOW as i32,
                // Longer than anything it is given, so it never splits speech itself.
                max_speech_duration: LONGEST as f32 / RATE as f32 + 1.0,
            },
            sample_rate: RATE as i32,
            num_threads: 1,
            provider: Some("cpu".into()),
            ..Default::default()
        })
    }
}

/// The Silero voice activity detector, loaded once and run in this process.
pub struct SileroVad {
    detector: VoiceActivityDetector,
}

impl SileroVad {
    /// Load the model.
    pub fn new(config: &SileroVadConfig) -> Result<Self, Error> {
        // The detector buffers the speech it has heard until it is taken, which is at most what it is given at once.
        let buffer = LONGEST as f32 / RATE as f32 + 1.0;
        Error::loaded(
            VoiceActivityDetector::create(&config.sherpa()?, buffer),
            format!(
                "the Silero voice activity model at {}",
                config.model.display()
            ),
        )
        .map(|detector| Self { detector })
    }

    /// Take the stretches of speech the detector has finished, relative to where it was reset.
    fn take(&self, speech: &mut Vec<Range<usize>>) {
        while let Some(segment) = self.detector.front() {
            let start = usize::try_from(segment.start()).unwrap_or(0);
            let length = usize::try_from(segment.n()).unwrap_or(0);
            speech.push(start..start + length);
            self.detector.pop();
        }
    }
}

impl VoiceActivity for SileroVad {
    fn speech(&mut self, samples: &[i16]) -> Result<Vec<Range<usize>>, Error> {
        if samples.len() > LONGEST {
            return Err(Error::Engine(format!(
                "the voice activity detector hears at most {} seconds at once",
                LONGEST / RATE as usize
            )));
        }
        self.detector.reset();
        let mut speech = Vec::new();
        for window in samples.chunks(WINDOW) {
            // The last window is padded with silence to a whole window, so none of it is left in the detector for the next call.
            let mut window: Vec<f32> = window
                .iter()
                .map(|&sample| f32::from(sample) / 32768.0)
                .collect();
            window.resize(WINDOW, 0.0);
            self.detector.accept_waveform(&window);
            self.take(&mut speech);
        }
        self.detector.flush();
        self.take(&mut speech);
        self.detector.reset();
        // Keep within the audio, without the padding, in order, as the cut expects.
        for range in &mut speech {
            range.end = range.end.min(samples.len());
            range.start = range.start.min(range.end);
        }
        speech.sort_by_key(|range| range.start);
        Ok(speech)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_a_missing_model() {
        let config = SileroVadConfig::from_root(Path::new("/nonexistent"));
        assert!(matches!(
            SileroVad::new(&config),
            Err(Error::Unavailable(message)) if message.contains("fetch-asr-model")
        ));
    }

    /// The LibriSpeech clip `name` from the `clips` that come with the Whisper model, at 16 kHz.
    fn clip(clips: &Path, name: &str) -> Vec<i16> {
        let path = clips.join(name);
        let wave = sherpa_onnx::Wave::read(path.to_str().unwrap()).expect("read the clip");
        assert_eq!(wave.sample_rate(), RATE as i32);
        wave.samples()
            .iter()
            .map(|sample| (sample.clamp(-1.0, 1.0) * 32767.0) as i16)
            .collect()
    }

    /// Skipped without the Silero model and the clips of the Whisper model the tests use; see [`crate::test_models`].
    #[test]
    fn hears_the_silence_between_two_clips() {
        use crate::{stt::WhisperConfig, test_models};

        let root = test_models::root();
        let config = SileroVadConfig::from_root(&root);
        let clips = WhisperConfig::named(&root, test_models::WHISPER_MODEL)
            .model
            .join("test_wavs");
        test_models::with(
            "the Silero voice activity model and the clips that come with the Whisper model",
            &format!(
                "bash scripts/fetch-asr-model.sh whisper {}",
                test_models::WHISPER_MODEL
            ),
            &[&config.model, &clips],
            || {
                let mut vad = SileroVad::new(&config).unwrap();
                let first = clip(&clips, "0.wav");
                let second = clip(&clips, "1.wav");
                let gap = RATE as usize;
                let mut samples = first.clone();
                samples.extend(std::iter::repeat_n(0, gap));
                samples.extend(&second);
                let speech = vad.speech(&samples).unwrap();
                assert!(!speech.is_empty());
                // No speech is heard in the middle of the gap, and some is heard on either side.
                let middle = first.len() + gap / 2;
                assert!(
                    speech.iter().all(|range| !range.contains(&middle)),
                    "{speech:?}"
                );
                assert!(speech.iter().any(|range| range.start < first.len()));
                assert!(speech.iter().any(|range| range.end > first.len() + gap));
                // Hearing the same audio again gives the same answer.
                assert_eq!(vad.speech(&samples).unwrap(), speech);
                // A whole segment of silence or faint noise holds no speech.
                assert!(vad.speech(&vec![0; MAX_SEGMENT]).unwrap().is_empty());
                let noise: Vec<i16> = (0..MAX_SEGMENT)
                    .map(|index| ((index * 7919 % 401) as i16 - 200) / 4)
                    .collect();
                assert!(vad.speech(&noise).unwrap().is_empty());
                let started = std::time::Instant::now();
                vad.speech(&samples).unwrap();
                eprintln!(
                    "Heard {:.1} seconds in {:.3} seconds.",
                    samples.len() as f64 / f64::from(RATE),
                    started.elapsed().as_secs_f64()
                );
            },
        );
    }

    #[test]
    fn a_model_sherpa_onnx_refuses_or_too_much_audio_is_said() {
        let config = SileroVadConfig::from_root(&crate::test_models::root());
        crate::test_models::with(
            "the Silero voice activity model",
            "bash scripts/fetch-asr-model.sh whisper tiny.en",
            &[&config.model],
            || {
                // sherpa-onnx refuses so high a threshold.
                let refused = SileroVadConfig {
                    threshold: 5.0,
                    ..config.clone()
                };
                let Err(Error::Unavailable(_)) = SileroVad::new(&refused) else {
                    unreachable!("the threshold is refused");
                };
                let mut vad = SileroVad::new(&config).unwrap();
                let Err(Error::Engine(_)) = vad.speech(&vec![0; LONGEST + 1]) else {
                    unreachable!("too much audio is refused");
                };
            },
        );
    }
}
