// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Streaming recognition in-process with a sherpa-onnx streaming transducer, such as NVIDIA's cache-aware streaming FastConformer.
//!
//! Each utterance has its own sherpa-onnx stream, which keeps the encoder's cache between pieces of audio, so every sample is decoded once, as it arrives. Greedy transducer search only appends to its hypothesis, so words already emitted do not change.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use sherpa_onnx::{
    OnlineModelConfig, OnlineRecognizer, OnlineRecognizerConfig, OnlineStream,
    OnlineTransducerModelConfig,
};

use super::{RATE, RecognitionStream, StreamingRecognizer, Transcript};
use crate::Error;

/// The default model, fetched by `scripts/fetch-asr-model.sh`.
pub const DEFAULT_MODEL: &str =
    "sherpa-onnx-nemo-streaming-fast-conformer-transducer-en-480ms-int8";

/// Silence appended when an utterance ends, so that the encoder's look-ahead covers its last word.
const TAIL_PADDING: usize = RATE as usize * 8 / 10;

/// Silence fed before an utterance's audio. Push-to-talk audio can start right on the first word, and without a lead-in the transducer often drops it.
const LEAD_PADDING: usize = RATE as usize * 3 / 10;

/// Where to find a streaming transducer model and how to run it.
#[derive(Clone, Debug)]
pub struct TransducerConfig {
    /// A directory holding `encoder*.onnx`, `decoder*.onnx`, `joiner*.onnx` and `tokens.txt`. Quantized `*.int8.onnx` files are preferred when both are present.
    pub model: PathBuf,
    /// The ONNX Runtime threads used for decoding.
    pub threads: u32,
    /// The ONNX Runtime execution provider: `cpu`, or `cuda` with the shared CUDA build of sherpa-onnx.
    pub provider: String,
}

impl TransducerConfig {
    /// The default model under the repository `root`: `.tools/asr/` followed by [`DEFAULT_MODEL`].
    pub fn from_root(root: &Path) -> Self {
        Self::with_model(root.join(".tools/asr").join(DEFAULT_MODEL))
    }

    /// A model in the directory `model`, decoded on the CPU with two threads.
    pub fn with_model(model: impl Into<PathBuf>) -> Self {
        Self {
            model: model.into(),
            threads: 2,
            provider: "cpu".into(),
        }
    }

    /// The model file whose name starts with `part`, preferring a quantized one.
    fn file(&self, part: &str) -> Option<String> {
        // Read once its tokens were found, the directory is there.
        let mut candidates: Vec<PathBuf> = std::fs::read_dir(&self.model)
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with(part) && name.ends_with(".onnx"))
            })
            .collect();
        candidates.sort_by_key(|path| !path.to_string_lossy().ends_with(".int8.onnx"));
        candidates
            .first()
            .map(|path| path.to_string_lossy().into_owned())
    }

    fn sherpa(&self) -> Result<OnlineRecognizerConfig, Error> {
        let missing = || {
            Error::Unavailable(format!(
                "the streaming recognition model is not installed in {}; run bash scripts/fetch-asr-model.sh",
                self.model.display()
            ))
        };
        let tokens = self.model.join("tokens.txt");
        if !tokens.exists() {
            return Err(missing());
        }
        let transducer = OnlineTransducerModelConfig {
            encoder: Some(self.file("encoder").ok_or_else(missing)?),
            decoder: Some(self.file("decoder").ok_or_else(missing)?),
            joiner: Some(self.file("joiner").ok_or_else(missing)?),
        };
        Ok(OnlineRecognizerConfig {
            model_config: OnlineModelConfig {
                transducer,
                tokens: Some(tokens.to_string_lossy().into_owned()),
                num_threads: i32::try_from(self.threads).unwrap_or(1).max(1),
                provider: Some(self.provider.clone()),
                ..Default::default()
            },
            decoding_method: Some("greedy_search".into()),
            // Push-to-talk delimits utterances, so the recognizer never ends one by itself.
            enable_endpoint: false,
            ..Default::default()
        })
    }
}

/// A [`StreamingRecognizer`] backed by a sherpa-onnx streaming transducer in this process.
pub struct TransducerRecognizer {
    recognizer: Arc<OnlineRecognizer>,
}

impl TransducerRecognizer {
    /// Load the model. This takes a few seconds, so it is done once, when the server starts.
    pub fn new(config: &TransducerConfig) -> Result<Self, Error> {
        let sherpa = config.sherpa()?;
        Error::loaded(
            OnlineRecognizer::create(&sherpa),
            format!(
                "the streaming recognition model in {}",
                config.model.display()
            ),
        )
        .map(|recognizer| Self {
            recognizer: Arc::new(recognizer),
        })
    }
}

impl StreamingRecognizer for TransducerRecognizer {
    fn open(&mut self) -> Result<Box<dyn RecognitionStream>, Error> {
        let stream = self.recognizer.create_stream();
        stream.accept_waveform(RATE as i32, &vec![0.0; LEAD_PADDING]);
        Ok(Box::new(TransducerStream {
            stream,
            recognizer: Arc::clone(&self.recognizer),
            text: String::new(),
        }))
    }
}

/// One utterance's sherpa-onnx stream.
struct TransducerStream {
    // Declared before `recognizer`, so the stream is destroyed before the recognizer it belongs to.
    stream: OnlineStream,
    recognizer: Arc<OnlineRecognizer>,
    text: String,
}

impl TransducerStream {
    fn decode_ready(&mut self) -> Result<(), Error> {
        while self.recognizer.is_ready(&self.stream) {
            self.recognizer.decode(&self.stream);
        }
        Error::produced(
            self.recognizer.get_result(&self.stream),
            "the streaming recognizer",
        )
        .map(|result| self.text = result.text.trim().to_owned())
    }
}

impl RecognitionStream for TransducerStream {
    fn accept(&mut self, samples: &[i16]) {
        let samples: Vec<f32> = samples
            .iter()
            .map(|&sample| f32::from(sample) / 32768.0)
            .collect();
        self.stream.accept_waveform(RATE as i32, &samples);
    }

    fn due(&self) -> bool {
        self.recognizer.is_ready(&self.stream)
    }

    fn decode(&mut self) -> Result<(), Error> {
        self.decode_ready()
    }

    fn transcript(&self) -> Transcript {
        split(&self.text)
    }

    fn finish(&mut self) -> Result<String, Error> {
        self.stream
            .accept_waveform(RATE as i32, &vec![0.0; TAIL_PADDING]);
        self.stream.input_finished();
        self.decode_ready().map(|()| self.text.clone())
    }
}

/// Split a hypothesis into the words that are complete and the last word, which may still grow as more of it is decoded.
fn split(text: &str) -> Transcript {
    match text.rsplit_once(char::is_whitespace) {
        Some((committed, tentative)) => Transcript {
            committed: committed.trim().to_owned(),
            tentative: tentative.to_owned(),
        },
        None => Transcript {
            committed: String::new(),
            tentative: text.to_owned(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Recognition, TranscriptEvent};
    use std::{
        sync::mpsc,
        time::{Duration, Instant},
    };

    #[test]
    fn splits_the_last_word_off_as_tentative() {
        assert_eq!(
            split("after early night"),
            Transcript {
                committed: "after early".into(),
                tentative: "night".into()
            }
        );
        assert_eq!(
            split("after"),
            Transcript {
                committed: String::new(),
                tentative: "after".into()
            }
        );
        assert_eq!(split(""), Transcript::default());
    }

    #[test]
    fn reports_a_missing_model() {
        let config = TransducerConfig::from_root(Path::new("/nonexistent"));
        assert!(matches!(
            TransducerRecognizer::new(&config),
            Err(Error::Unavailable(message)) if message.contains("fetch-asr-model")
        ));
    }

    /// Speaks sentences with Kokoro and recognizes them as they are produced, as push-to-talk would deliver them. Skipped without both models; see [`crate::test_models`].
    #[test]
    fn recognizes_synthesized_speech_while_it_arrives() {
        use crate::{
            test_models,
            tts::{KokoroConfig, KokoroSynthesizer, Synthesizer},
        };

        let root = test_models::root();
        let voice = KokoroConfig::from_root(&root);
        let config = TransducerConfig::from_root(&root);
        test_models::with(
            "the Kokoro and streaming recognition models",
            "bash scripts/fetch-asr-model.sh and bash scripts/fetch-tts-model.sh",
            &[&voice.model, &config.model],
            || {
                let mut kokoro = KokoroSynthesizer::new(&voice).unwrap();
                let mut audio = Vec::new();
                let mut rate = 0;
                kokoro
                    .synthesize(
                        "The quick brown fox jumps over the lazy dog. Then it runs back home.",
                        &mut |chunk| {
                            rate = chunk.sample_rate;
                            audio.extend(chunk.samples);
                        },
                    )
                    .unwrap();

                let recognizer = TransducerRecognizer::new(&config).unwrap();
                let mut recognition = Recognition::spawn(recognizer);
                let (sender, events) = mpsc::channel();
                let mut utterance = recognition.begin(1, rate, move |event| {
                    let _ = sender.send(event);
                });
                // 100 ms frames in real time, as the headset sends them.
                let start = Instant::now();
                let mut partials = 0;
                for (index, frame) in audio.chunks(rate as usize / 10).enumerate() {
                    utterance.push(frame);
                    while let Ok(event) = events.try_recv() {
                        let TranscriptEvent::Partial(_) = event else {
                            unreachable!("unexpected {event:?} before the utterance ended");
                        };
                        partials += 1;
                    }
                    let due = start + Duration::from_millis(100 * (index as u64 + 1));
                    std::thread::sleep(due.saturating_duration_since(Instant::now()));
                }
                // Partial results arrive while the participant is still speaking.
                assert!(partials > 0, "no partial result before the utterance ended");
                utterance.end();
                // Partial results may still arrive before the final one.
                let text = std::iter::from_fn(|| events.recv_timeout(Duration::from_secs(60)).ok())
                    .find_map(TranscriptEvent::outcome)
                    .expect("the utterance ends")
                    .unwrap()
                    .to_lowercase();
                assert!(text.contains("quick brown fox"), "{text}");
                assert!(text.contains("back home"), "{text}");
            },
        );
    }

    /// A model directory of its own, with `files`, removed when dropped.
    struct Model(std::path::PathBuf);

    impl Model {
        fn with(name: &str, files: &[&str]) -> Self {
            let directory = std::env::temp_dir()
                .join(format!("tomori-transducer-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&directory);
            std::fs::create_dir_all(&directory).unwrap();
            for file in files {
                std::fs::write(directory.join(file), "").unwrap();
            }
            Self(directory)
        }
    }

    impl Drop for Model {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_quantized_model_file_is_preferred_and_a_missing_one_is_said() {
        let both = Model::with("both", &["tokens.txt", "encoder.onnx", "encoder.int8.onnx"]);
        let config = TransducerConfig::with_model(&both.0);
        assert!(
            config
                .file("encoder")
                .unwrap()
                .ends_with("encoder.int8.onnx")
        );
        for (name, files) in [
            ("encoder", &["tokens.txt"][..]),
            ("decoder", &["tokens.txt", "encoder.onnx"][..]),
            (
                "joiner",
                &["tokens.txt", "encoder.onnx", "decoder.onnx"][..],
            ),
        ] {
            let model = Model::with(name, files);
            let Err(Error::Unavailable(_)) = TransducerConfig::with_model(&model.0).sherpa() else {
                unreachable!("a model without its {name} is not installed");
            };
        }
    }
}
