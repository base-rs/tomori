// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Whole-clip Whisper recognition in-process through sherpa-onnx, used by the server's offline backend and by the recognition benchmark to compare accuracy and latency with the streaming recognizer.
//!
//! Whisper cannot stream: it decodes a complete clip of at most 30 seconds, so the server uses it only through [`super::OfflineRecognition`], which splits longer clips at their pauses.

use std::path::{Path, PathBuf};

use sherpa_onnx::{OfflineRecognizer, OfflineRecognizerConfig, OfflineWhisperModelConfig};

use super::RATE;
use crate::Error;

/// The Whisper model the server's offline backend uses unless configured otherwise.
pub const DEFAULT_WHISPER_MODEL: &str = "small.en";

/// Where to find a sherpa-onnx Whisper model and how to run it.
#[derive(Clone, Debug)]
pub struct WhisperConfig {
    /// The model's directory, such as `sherpa-onnx-whisper-small.en`, holding `small.en-encoder.int8.onnx`, `small.en-decoder.int8.onnx` and `small.en-tokens.txt`.
    pub model: PathBuf,
    /// The prefix of the model's file names, such as `small.en`.
    pub name: String,
    pub language: String,
    /// The ONNX Runtime threads used for decoding.
    pub threads: u32,
    /// The ONNX Runtime execution provider: `cpu`, or `cuda` with the shared CUDA build of sherpa-onnx.
    pub provider: String,
}

impl WhisperConfig {
    /// The default model, [`DEFAULT_WHISPER_MODEL`], fetched by `scripts/fetch-asr-model.sh whisper` under the repository `root`: `.tools/asr/sherpa-onnx-whisper-small.en`.
    pub fn from_root(root: &Path) -> Self {
        Self::named(root, DEFAULT_WHISPER_MODEL)
    }

    /// The Whisper model `name`, such as `small.en` or `tiny.en`, fetched by `scripts/fetch-asr-model.sh whisper NAME` under the repository `root`: `.tools/asr/sherpa-onnx-whisper-NAME`. Use [`Self::valid_name`] to check a name from outside the server first.
    pub fn named(root: &Path, name: &str) -> Self {
        Self {
            model: root
                .join(".tools/asr")
                .join(format!("sherpa-onnx-whisper-{name}")),
            name: name.into(),
            language: "en".into(),
            threads: 4,
            provider: "cpu".into(),
        }
    }

    /// Whether `name` can name a model: lowercase letters, digits, `.` and `-`, not starting with `.`, so that it stays within `.tools/asr`.
    pub fn valid_name(name: &str) -> bool {
        !name.is_empty()
            && !name.starts_with('.')
            && name.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || b".-".contains(&byte)
            })
    }

    fn sherpa(&self) -> Result<OfflineRecognizerConfig, Error> {
        let file = |suffix: &str| self.model.join(format!("{}-{suffix}", self.name));
        let files = [
            file("encoder.int8.onnx"),
            file("decoder.int8.onnx"),
            file("tokens.txt"),
        ];
        if !files.iter().all(|file| file.exists()) {
            return Err(Error::Unavailable(format!(
                "the Whisper model {} is not installed in {}; run bash scripts/fetch-asr-model.sh whisper {}",
                self.name,
                self.model.display(),
                self.name
            )));
        }
        let [encoder, decoder, tokens] =
            files.map(|file| Some(file.to_string_lossy().into_owned()));
        let mut config = OfflineRecognizerConfig::default();
        config.model_config.whisper = OfflineWhisperModelConfig {
            encoder,
            decoder,
            language: Some(self.language.clone()),
            task: Some("transcribe".into()),
            tail_paddings: -1,
            ..Default::default()
        };
        config.model_config.tokens = tokens;
        config.model_config.num_threads = i32::try_from(self.threads).unwrap_or(1).max(1);
        config.model_config.provider = Some(self.provider.clone());
        Ok(config)
    }
}

/// Whisper, loaded once and run in this process.
pub struct WhisperRecognizer {
    recognizer: OfflineRecognizer,
}

impl WhisperRecognizer {
    /// Load the model.
    pub fn new(config: &WhisperConfig) -> Result<Self, Error> {
        let recognizer = OfflineRecognizer::create(&config.sherpa()?).ok_or_else(|| {
            Error::Unavailable(format!(
                "sherpa-onnx could not load the Whisper model in {}",
                config.model.display()
            ))
        })?;
        Ok(Self { recognizer })
    }

    /// Transcribe `audio`, 16 kHz mono and at most 30 seconds long.
    pub fn transcribe(&mut self, audio: &[i16]) -> Result<String, Error> {
        if audio.len() > RATE as usize * 30 {
            return Err(Error::Engine("Whisper decodes at most 30 seconds".into()));
        }
        let samples: Vec<f32> = audio
            .iter()
            .map(|&sample| f32::from(sample) / 32768.0)
            .collect();
        let stream = self.recognizer.create_stream();
        stream.accept_waveform(RATE as i32, &samples);
        self.recognizer.decode(&stream);
        let result = stream
            .get_result()
            .ok_or_else(|| Error::Engine("Whisper returned no result".into()))?;
        Ok(result.text.trim().to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_a_missing_installation() {
        let config = WhisperConfig::named(Path::new("/nonexistent"), "tiny.en");
        assert!(matches!(
            WhisperRecognizer::new(&config),
            Err(Error::Unavailable(message)) if message.contains("fetch-asr-model.sh whisper tiny.en")
        ));
    }

    #[test]
    fn names_its_model() {
        let root = Path::new("/srv/tomori");
        let config = WhisperConfig::from_root(root);
        assert_eq!(config.name, DEFAULT_WHISPER_MODEL);
        assert_eq!(
            config.model,
            root.join(".tools/asr/sherpa-onnx-whisper-small.en")
        );
        let config = WhisperConfig::named(root, "tiny.en");
        assert_eq!(config.name, "tiny.en");
        assert_eq!(
            config.model,
            root.join(".tools/asr/sherpa-onnx-whisper-tiny.en")
        );
        for name in ["small.en", "tiny.en", "large-v3"] {
            assert!(WhisperConfig::valid_name(name), "{name:?}");
        }
        for name in ["", ".", "..", "../small.en", "a/b", "Small.en", "tiny en"] {
            assert!(!WhisperConfig::valid_name(name), "{name:?}");
        }
    }

    /// Skipped without the Whisper model the tests use; see [`crate::test_models`].
    #[test]
    fn decodes_silence_without_failing() {
        use crate::test_models;

        let config = WhisperConfig::named(&test_models::root(), test_models::WHISPER_MODEL);
        if !test_models::installed(
            "the Whisper model",
            &format!("bash scripts/fetch-asr-model.sh whisper {}", config.name),
            &[&config.model],
        ) {
            return;
        }
        let mut whisper = WhisperRecognizer::new(&config).unwrap();
        assert!(whisper.transcribe(&vec![0; RATE as usize]).is_ok());
        assert!(matches!(
            whisper.transcribe(&vec![0; RATE as usize * 31]),
            Err(Error::Engine(_))
        ));
    }
}
