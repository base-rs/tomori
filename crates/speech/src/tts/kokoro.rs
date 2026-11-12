// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Kokoro synthesis in-process through sherpa-onnx, on a thread owned by the synthesizer.
//!
//! Text is turned into Kokoro's phonemes in Rust, by a [`Phonemizer`], and sherpa-onnx is given the phonemes, so that no GPL code is linked: sherpa-onnx 2 no longer pronounces text itself with espeak-ng.
use std::{
    cell::Cell,
    collections::HashSet,
    panic::{self, AssertUnwindSafe},
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread::{self, JoinHandle},
};

use sherpa_onnx::{
    GeneratedAudio, GenerationConfig, OfflineTts, OfflineTtsConfig, OfflineTtsKokoroModelConfig,
    OfflineTtsModelConfig,
};

use super::{
    AudioChunk, Synthesizer, UNAVAILABLE,
    english::English,
    phonemes::{self, Phonemizer},
};
use crate::{
    Error, Stopper,
    stop::{Exited, panic_message},
};

/// Why requests fail while the server shuts down.
const SHUTTING_DOWN: &str = "the server is shutting down";

/// The number of voices in the Kokoro multi-language v1.0 model.
const VOICES: u32 = 54;

/// The longest segment accepted, in characters. Segments come from [`crate::Speakable`], which yields sentences far shorter than this.
const TEXT_LIMIT: usize = 2000;

/// The files of the `kokoro-multi-lang-v1_0` model that synthesis uses. Its lexicons and `espeak-ng-data` are not used: text is phonemized in Rust.
const FILES: [&str; 3] = ["model.onnx", "voices.bin", "tokens.txt"];

/// Where to find the model and how to speak.
#[derive(Clone, Debug)]
pub struct KokoroConfig {
    /// The `kokoro-multi-lang-v1_0` directory: `model.onnx`, `voices.bin` and `tokens.txt`.
    pub model: PathBuf,
    /// The speaker id, 0 to 53.
    pub voice: u32,
    pub speed: f32,
    /// The ONNX Runtime threads used for synthesis.
    pub threads: u32,
    /// A file of pronunciations that add to and replace Tomori's own, in the form of `assets/pronunciations.toml`, if it exists; the server's is `pronunciations.toml` in its state directory.
    pub pronunciations: Option<PathBuf>,
}

impl KokoroConfig {
    /// The model fetched by `scripts/fetch-tts-model.sh` under the repository `root`: `.tools/tts/kokoro-multi-lang-v1_0`.
    pub fn from_root(root: &Path) -> Self {
        Self {
            model: root.join(".tools/tts/kokoro-multi-lang-v1_0"),
            // `af_heart`.
            voice: 3,
            speed: 1.0,
            threads: 2,
            pronunciations: None,
        }
    }

    /// Check the configuration and the installation, and return the phonemes the model knows.
    fn check(&self) -> Result<HashSet<char>, Error> {
        if self.voice >= VOICES {
            return Err(Error::Unavailable(format!(
                "Kokoro voice {} is out of range 0 to {}",
                self.voice,
                VOICES - 1
            )));
        }
        if !FILES.iter().all(|file| self.model.join(file).exists()) {
            return Err(Error::Unavailable(format!(
                "the Kokoro model is not installed in {}; run bash scripts/fetch-tts-model.sh",
                self.model.display()
            )));
        }
        let tokens = self.model.join("tokens.txt");
        let tokens = std::fs::read_to_string(&tokens).map_err(|error| {
            Error::Unavailable(format!(
                "Kokoro's phonemes could not be read from {}: {error}",
                tokens.display()
            ))
        })?;
        Ok(phonemes::symbols(&tokens))
    }

    fn sherpa(&self) -> OfflineTtsConfig {
        let file = |name: &str| Some(self.model.join(name).to_string_lossy().into_owned());
        OfflineTtsConfig {
            model: OfflineTtsModelConfig {
                kokoro: OfflineTtsKokoroModelConfig {
                    model: file("model.onnx"),
                    voices: file("voices.bin"),
                    tokens: file("tokens.txt"),
                    // Each segment is given as phonemes, so no lexicon is loaded.
                    ..Default::default()
                },
                num_threads: i32::try_from(self.threads).unwrap_or(1).max(1),
                provider: Some("cpu".into()),
                debug: false,
                ..Default::default()
            },
            // Generate and deliver one sentence at a time, so the first sentence of a segment is heard before the rest is synthesized; sherpa-onnx ignores any other value for Kokoro.
            max_num_sentences: 1,
            ..Default::default()
        }
    }
}

/// What the synthesis thread asks of the Kokoro model: [`OfflineTts`], or a stand-in in tests.
trait Engine {
    /// A segment's audio.
    type Audio;

    fn sample_rate(&self) -> i32;
    fn num_speakers(&self) -> i32;
    /// Synthesize the sentences of phonemes that `generation` gives, passing each sentence's samples to `callback`, which returns whether to go on, and return the segment's audio, or `None` if it could not be synthesized.
    fn generate(
        &self,
        generation: &GenerationConfig,
        callback: impl FnMut(&[f32], f32) -> bool + 'static,
    ) -> Option<Self::Audio>;
    fn samples(audio: &Self::Audio) -> &[f32];
}

impl Engine for OfflineTts {
    type Audio = GeneratedAudio;

    fn sample_rate(&self) -> i32 {
        OfflineTts::sample_rate(self)
    }

    fn num_speakers(&self) -> i32 {
        OfflineTts::num_speakers(self)
    }

    fn generate(
        &self,
        generation: &GenerationConfig,
        callback: impl FnMut(&[f32], f32) -> bool + 'static,
    ) -> Option<GeneratedAudio> {
        // The text is given as phonemes instead.
        self.generate_with_config("", generation, Some(callback))
    }

    fn samples(audio: &GeneratedAudio) -> &[f32] {
        audio.samples()
    }
}

/// How the synthesis thread reads text: in phonemes, of which the model knows `symbols`.
struct Reading<P> {
    phonemizer: P,
    symbols: HashSet<char>,
}

/// What the synthesis thread reports for a request.
enum Message {
    Audio(Vec<f32>),
    Done,
    Failed(Error),
}

struct Request {
    text: String,
    voice: i32,
    speed: f32,
    replies: Sender<Message>,
}

enum Command {
    Synthesize(Request),
    /// Unload the model and exit, once the requests queued before this one are answered.
    Stop,
}

/// A [`Synthesizer`] that runs Kokoro in this process.
///
/// The model is loaded and run on a thread owned by the synthesizer, so synthesis never runs on the caller's scheduler. Each sentence's audio is passed to the caller as soon as it is generated. A segment that fails is reported to its caller and not retried. A panic on the thread disables synthesis: it is logged once, and every later segment fails with [`Error::Stopped`].
pub struct KokoroSynthesizer {
    commands: Sender<Command>,
    thread: Option<JoinHandle<()>>,
    exited: Exited,
    /// Set by a [`Stopper`]: synthesis stops at the next sentence, and waiting requests fail.
    stopping: Arc<AtomicBool>,
    voice: u32,
    speed: f32,
    sample_rate: u32,
    speakers: u32,
}

impl KokoroSynthesizer {
    /// Load the model on a new synthesis thread, waiting until it is ready so that the first segment does not wait for it.
    pub fn new(config: &KokoroConfig) -> Result<Self, Error> {
        let symbols = config.check()?;
        let sherpa = config.sherpa();
        let pronunciations = config.pronunciations.clone();
        Self::start(
            move || {
                let phonemizer = English::with_pronunciations(pronunciations.as_deref());
                OfflineTts::create(&sherpa).map(|tts| {
                    (
                        tts,
                        Reading {
                            phonemizer,
                            symbols,
                        },
                    )
                })
            },
            config.voice,
            config.speed,
        )
    }

    /// Load the model and its phonemizer with `load` on a new synthesis thread, as [`new`](Self::new) does, to speak with `voice` at `speed`.
    fn start<E: Engine + 'static, P: Phonemizer + 'static>(
        load: impl FnOnce() -> Option<(E, Reading<P>)> + Send + 'static,
        voice: u32,
        speed: f32,
    ) -> Result<Self, Error> {
        let (ready_sender, ready) = mpsc::channel();
        let (commands, receiver) = mpsc::channel();
        let exited = Exited::default();
        let guard = exited.guard();
        let stopping = Arc::new(AtomicBool::new(false));
        let serving = Arc::clone(&stopping);
        let thread = thread::Builder::new()
            .name("speech-kokoro".into())
            .spawn(move || {
                // Dropped last, once the model has been unloaded.
                let _guard = guard;
                let Some((tts, reading)) = load() else {
                    let _ = ready_sender.send(Err(Error::Unavailable(
                        "sherpa-onnx could not load the Kokoro model".into(),
                    )));
                    return;
                };
                let _ = ready_sender.send(Ok((tts.sample_rate(), tts.num_speakers())));
                serve(&tts, &reading, receiver, &serving);
            })
            // As the other engine threads, a thread the system cannot start is not survived.
            .expect("spawn the Kokoro thread");
        let (sample_rate, speakers) = match ready.recv() {
            Ok(Ok(description)) => description,
            Ok(Err(error)) => {
                let _ = thread.join();
                return Err(error);
            }
            Err(_) => {
                let _ = thread.join();
                return Err(Error::Unavailable(
                    "the Kokoro thread stopped while loading the model".into(),
                ));
            }
        };
        let synthesizer = Self {
            commands,
            thread: Some(thread),
            exited,
            stopping,
            voice,
            speed,
            sample_rate: u32::try_from(sample_rate).unwrap_or(0),
            speakers: u32::try_from(speakers).unwrap_or(0),
        };
        if synthesizer.sample_rate == 0 {
            return Err(Error::Unavailable(
                "the Kokoro model reports no sample rate".into(),
            ));
        }
        if synthesizer.voice >= synthesizer.speakers {
            return Err(Error::Unavailable(format!(
                "Kokoro voice {} is out of range; the model has {} voices",
                synthesizer.voice, synthesizer.speakers
            )));
        }
        Ok(synthesizer)
    }

    /// The sample rate of the audio produced, 24 kHz for Kokoro.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// The number of voices the model has.
    pub fn speakers(&self) -> u32 {
        self.speakers
    }

    /// A handle that stops the synthesis thread: the segment being synthesized stops after its current sentence, waiting segments fail with [`Error::Stopped`], the model is unloaded, and the thread exits.
    pub fn stopper(&self) -> Stopper {
        let stopping = Arc::clone(&self.stopping);
        let commands = self.commands.clone();
        Stopper::new(
            move || {
                stopping.store(true, Ordering::SeqCst);
                let _ = commands.send(Command::Stop);
            },
            self.exited.clone(),
        )
    }
}

/// Serve requests on the synthesis thread until the synthesizer is dropped or stopped, or a panic disables it.
fn serve<E: Engine, P: Phonemizer>(
    tts: &E,
    reading: &Reading<P>,
    commands: Receiver<Command>,
    stopping: &Arc<AtomicBool>,
) {
    for command in commands {
        let Command::Synthesize(request) = command else {
            return;
        };
        if stopping.load(Ordering::SeqCst) {
            let _ = request
                .replies
                .send(Message::Failed(Error::Stopped(SHUTTING_DOWN.into())));
            continue;
        }
        match panic::catch_unwind(AssertUnwindSafe(|| {
            generate(tts, reading, &request, stopping)
        })) {
            Ok(reply) => {
                let _ = request.replies.send(reply);
            }
            Err(panic) => {
                log::error!(
                    "Kokoro speech synthesis stopped after a panic ({}); it is unavailable until the server restarts.",
                    panic_message(&*panic)
                );
                // Later requests find the thread gone and fail with the same error.
                let _ = request
                    .replies
                    .send(Message::Failed(Error::Stopped(UNAVAILABLE.into())));
                return;
            }
        }
    }
}

/// Synthesize one request, streaming its audio, and return the final reply.
fn generate<E: Engine, P: Phonemizer>(
    tts: &E,
    reading: &Reading<P>,
    request: &Request,
    stopping: &Arc<AtomicBool>,
) -> Message {
    let sentences = phonemes::codepoints(
        reading.phonemizer.sentences(&request.text),
        &reading.symbols,
    );
    // Nothing to pronounce, such as an emoji alone, is silent, as code is.
    if sentences.is_empty() {
        return Message::Done;
    }
    let replies = request.replies.clone();
    let streamed = Rc::new(Cell::new(false));
    let callback = {
        let streamed = Rc::clone(&streamed);
        let stopping = Arc::clone(stopping);
        // This runs inside sherpa-onnx, where a panic would abort the process; nothing here panics.
        move |samples: &[f32], _progress: f32| {
            if stopping.load(Ordering::SeqCst) {
                return false;
            }
            if samples.is_empty() {
                return true;
            }
            streamed.set(true);
            // Stop early if the caller has gone away.
            replies.send(Message::Audio(samples.to_vec())).is_ok()
        }
    };
    let generation = GenerationConfig {
        sid: request.voice,
        speed: request.speed,
        phoneme_codepoints: Some(sentences),
        ..Default::default()
    };
    let audio = tts.generate(&generation, callback);
    if stopping.load(Ordering::SeqCst) {
        return Message::Failed(Error::Stopped(SHUTTING_DOWN.into()));
    }
    match audio {
        None => Message::Failed(Error::Engine("Kokoro could not synthesize the text".into())),
        Some(audio) if !streamed.get() => {
            // A build that does not call back still returns the whole segment.
            let samples = E::samples(&audio);
            if samples.is_empty() {
                Message::Failed(Error::Engine("Kokoro produced no audio".into()))
            } else {
                let _ = request.replies.send(Message::Audio(samples.to_vec()));
                Message::Done
            }
        }
        Some(_) => Message::Done,
    }
}

/// Convert samples from -1 to 1 into 16-bit PCM.
fn to_pcm(samples: &[f32]) -> Vec<i16> {
    samples
        .iter()
        .map(|sample| (sample.clamp(-1.0, 1.0) * 32767.0).round() as i16)
        .collect()
}

/// The text Kokoro is given: trimmed, without NUL characters, which sherpa-onnx cannot accept.
fn prepare(text: &str) -> Result<String, Error> {
    let text = text.replace('\0', " ");
    let text = text.trim();
    if text.is_empty() || text.chars().count() > TEXT_LIMIT {
        return Err(Error::Engine(format!(
            "speech requires 1 to {TEXT_LIMIT} characters"
        )));
    }
    Ok(text.to_owned())
}

impl Synthesizer for KokoroSynthesizer {
    fn synthesize(&mut self, text: &str, output: &mut dyn FnMut(AudioChunk)) -> Result<(), Error> {
        let text = prepare(text)?;
        let stopped = || Error::Stopped(UNAVAILABLE.into());
        let (replies, receiver) = mpsc::channel();
        let request = Request {
            text,
            voice: i32::try_from(self.voice).expect("the voice was checked against the model"),
            speed: self.speed,
            replies,
        };
        self.commands
            .send(Command::Synthesize(request))
            .map_err(|_| stopped())?;
        loop {
            // A request the thread drops unanswered, as it stops, failed as synthesis did.
            match receiver.recv().unwrap_or(Message::Failed(stopped())) {
                Message::Audio(samples) => output(AudioChunk {
                    sample_rate: self.sample_rate,
                    samples: to_pcm(&samples),
                }),
                Message::Done => return Ok(()),
                Message::Failed(error) => return Err(error),
            }
        }
    }
}

impl Drop for KokoroSynthesizer {
    /// Waits for the segments already queued, then unloads the model.
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Stop);
        let thread = self.thread.take().expect("joined only here");
        let _ = thread.join();
    }
}

#[cfg(test)]
mod tests;
