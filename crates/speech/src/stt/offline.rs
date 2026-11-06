// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Whole-clip speech recognition for push-to-talk utterances.
//!
//! The [`OfflineRecognizer`] decodes a complete clip, once, so it is more accurate than the streaming [`super::StreamingRecognizer`]. The cost is latency: nothing is reported until the participant stops speaking, and the final transcript arrives once the whole clip is decoded, a couple of seconds after they let go, instead of almost at once.
//!
//! [`OfflineRecognition`] owns one recognizer on a dedicated thread. Each utterance is driven through an [`OfflineUtterance`]: audio is buffered and resampled to 16 kHz, and when the participant stops the clip is decoded and reported through the utterance's sink as exactly one [`TranscriptEvent::Final`] or [`TranscriptEvent::Failed`]. There are no partial events.
//!
//! The recognizer decodes at most [`super::MAX_SEGMENT`] at once, so a longer clip is split at its pauses into segments that fit, which are decoded in order and their transcripts joined; see [`super::segment`]. What Whisper writes for a segment without speech, bracketed notes or a word or two the voice activity detector heard no speech for, is left out, so a clip with no speech in it at all is reported as an empty transcript. Utterances that have ended take turns, a segment at a time, so an utterance that ends while a long one is being decoded waits for at most one of its segments. An utterance longer than [`MAX_UTTERANCE`] fails with [`Error::TooLong`] as soon as it passes the limit.
//!
//! Once an utterance has ended, its sink is told when it waits behind another utterance, with [`Progress::Waiting`], and when it is transcribed after waiting, or in more than one segment, with [`Progress::Transcribing`] for each segment, so that the participant can tell recognition working from their words still on their way. An utterance that is transcribed at once, in one piece, is told only its outcome.
//!
//! A panic in the recognizer disables recognition until the process restarts: it is logged once, and every open and later utterance fails with [`Error::Stopped`]. A panic in the voice activity detector disables only the detector, and long utterances are then split by loudness alone.
use std::{
    collections::{HashMap, VecDeque},
    ops::Range,
    panic::{self, AssertUnwindSafe},
    sync::{
        Arc, Mutex, PoisonError,
        mpsc::{self, Receiver, Sender, TryRecvError},
    },
    thread::{self, JoinHandle},
    time::Instant,
};

use tomori_protocol::AudioFrame;

use crate::{
    Error, Stopper,
    stop::{Exited, panic_message},
    stt::{
        Progress, RATE, Resampler, TranscriptEvent,
        segment::{self, Cut, VoiceActivity},
    },
};

/// The most audio, in samples at [`RATE`], that one utterance may buffer: 5 minutes, about 10 MB.
///
/// It bounds the memory each utterance holds, and the time it can keep the recognizer busy. An utterance that would buffer more fails visibly as soon as it does, rather than having its end cut off.
pub const MAX_UTTERANCE: usize = RATE as usize * 60 * 5;

/// Why an utterance fails once recognition has stopped: a panic disabled it.
const STOPPED_MESSAGE: &str = "speech recognition stopped after an internal error";

/// An utterance's sink, shared by the decode thread and the utterance's handle, which tells it that it waits as it ends.
type Sink = Arc<Mutex<Box<dyn FnMut(TranscriptEvent) + Send>>>;

/// Tell `sink` of `event`.
fn tell(sink: &Sink, event: TranscriptEvent) {
    (sink.lock().unwrap_or_else(PoisonError::into_inner))(event);
}

/// Every utterance begun and without an outcome yet, and whether it has ended, shared by the decode thread and the handles, so that one ending can tell whether it waits behind another.
type Pending = Arc<Mutex<HashMap<u64, bool>>>;

fn pending(pending: &Pending) -> std::sync::MutexGuard<'_, HashMap<u64, bool>> {
    pending.lock().unwrap_or_else(PoisonError::into_inner)
}

/// An offline speech recognition engine, which decodes a complete clip, once. It cannot see a clip that is still being spoken, so it reports nothing while the participant is speaking.
pub trait OfflineRecognizer: Send {
    /// Transcribe the complete 16 kHz mono clip, at most [`super::MAX_SEGMENT`] long.
    fn transcribe(&mut self, samples: &[i16]) -> Result<String, Error>;
}

impl<R: OfflineRecognizer + ?Sized> OfflineRecognizer for Box<R> {
    fn transcribe(&mut self, samples: &[i16]) -> Result<String, Error> {
        (**self).transcribe(samples)
    }
}

enum Command {
    Begin {
        key: u64,
        sink: Sink,
    },
    Audio {
        key: u64,
        samples: Vec<i16>,
    },
    /// The participant stopped speaking `at`; `waiting` if the utterance was told that it waits behind another.
    End {
        key: u64,
        at: Instant,
        waiting: bool,
    },
    Cancel {
        key: u64,
    },
    /// Fail every utterance and exit at once.
    Stop,
}

/// Finds where each segment of a long clip ends, hearing its pauses with a voice activity detector when there is one.
struct Segmenter {
    voice: Option<Box<dyn VoiceActivity>>,
}

impl Segmenter {
    /// Where the segment of `samples` that starts at `start` ends.
    fn cut(&mut self, samples: &[i16], start: usize) -> Cut {
        let speech: Vec<Range<usize>> = segment::listen(samples, start)
            .and_then(|range| {
                let found = self.speech(&samples[range.clone()])?;
                Some(
                    found
                        .into_iter()
                        .map(|found| found.start + range.start..found.end + range.start)
                        .collect(),
                )
            })
            .unwrap_or_default();
        segment::cut(samples, start, &speech)
    }

    /// Whether the detector hears no speech at all in `samples`; false without a detector, or if it fails.
    fn silent(&mut self, samples: &[i16]) -> bool {
        self.speech(samples).is_some_and(|speech| speech.is_empty())
    }

    /// The stretches of `samples` that the detector hears as speech, or `None` without a detector, or if it fails.
    fn speech(&mut self, samples: &[i16]) -> Option<Vec<Range<usize>>> {
        let voice = self.voice.as_mut()?;
        match panic::catch_unwind(AssertUnwindSafe(|| voice.speech(samples))) {
            Ok(Ok(speech)) => Some(speech),
            Ok(Err(error)) => {
                log::warn!(
                    "Voice activity detection failed ({error}); this segment is placed by loudness alone."
                );
                None
            }
            Err(panic) => {
                log::error!(
                    "Voice activity detection stopped after a panic ({}); long utterances are split by loudness alone until the server restarts.",
                    panic_message(&*panic)
                );
                self.voice = None;
                None
            }
        }
    }
}

/// Whether `text` is only notes of what Whisper heard instead of speech, one or more, such as "[BLANK_AUDIO]", "(water running)" or "[Music] (static)", which it writes for a clip with no words in it.
fn annotation(text: &str) -> bool {
    let mut rest = text.trim();
    let mut notes = 0;
    while let Some(after) = rest.strip_prefix(['[', '(']) {
        let close = if rest.starts_with('[') { ']' } else { ')' };
        // A note holds no other brackets, so "(a) and (b)" is words, not notes.
        match after.find(['[', ']', '(', ')']) {
            Some(end) if after[end..].starts_with(close) => {
                rest = after[end + 1..].trim_start();
                notes += 1;
            }
            _ => return false,
        }
    }
    notes > 0 && rest.is_empty()
}

/// One utterance being buffered for a whole-clip decode.
struct OfflineEntry {
    key: u64,
    sink: Sink,
    /// The audio of the utterance, resampled to 16 kHz.
    samples: Vec<i16>,
    /// When the participant stopped speaking, or when the utterance's last segment was decoded: it waits for its next segment from then. `None` while the participant is still speaking.
    ready: Option<Instant>,
    /// Where the next segment to decode starts; the audio before it has been decoded.
    decoded: usize,
    /// Where each segment ends, found once its first segment is due to be decoded.
    cuts: Vec<Cut>,
    /// What its sink was last told of its progress.
    told: Told,
    /// The transcript of each segment decoded so far, in order, without the empty ones and those left out as having no speech.
    texts: Vec<String>,
    /// How many segments have been decoded, how many of them ended at their limit because there was no pause, and how many were left out of the transcript as having no speech in them.
    segments: usize,
    hard: usize,
    skipped: usize,
}

/// What an utterance that has ended was last told of its progress.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Told {
    /// Nothing: it was to be transcribed at once.
    Nothing,
    Waiting,
    Transcribing,
}

struct OfflineEngine<R: OfflineRecognizer> {
    /// The recognizer, until a panic disables it or the engine stops.
    recognizer: Option<R>,
    segmenter: Segmenter,
    commands: Receiver<Command>,
    /// Buffered utterances, in the order they began. Those that have ended are decoded a segment at a time, in turn; see [`OfflineEngine::step`].
    active: VecDeque<OfflineEntry>,
    /// A panic has disabled recognition; every later utterance fails at once.
    disabled: bool,
    /// The most samples an utterance may buffer: [`MAX_UTTERANCE`], except in tests.
    limit: usize,
    pending: Pending,
}

impl<R: OfflineRecognizer> OfflineEngine<R> {
    fn new(
        commands: Receiver<Command>,
        recognizer: R,
        voice: Option<Box<dyn VoiceActivity>>,
        limit: usize,
        pending: Pending,
    ) -> Self {
        Self {
            recognizer: Some(recognizer),
            segmenter: Segmenter { voice },
            commands,
            active: VecDeque::new(),
            disabled: false,
            limit,
            pending,
        }
    }

    fn run(mut self) {
        loop {
            match panic::catch_unwind(AssertUnwindSafe(|| self.turn())) {
                Ok(true) => {}
                Ok(false) => return,
                Err(panic) => self.disable(&panic_message(&*panic)),
            }
        }
    }

    /// Apply the waiting commands, waiting for one if there is nothing to decode, then decode one segment of an utterance that has ended; see [`OfflineEngine::step`]. Returns whether the engine keeps running.
    fn turn(&mut self) -> bool {
        if !self.active.iter().any(|entry| entry.ready.is_some()) {
            match self.commands.recv() {
                Ok(command) => {
                    if !self.apply(command) {
                        return false;
                    }
                }
                Err(_) => {
                    self.drain();
                    return false;
                }
            }
        }
        loop {
            match self.commands.try_recv() {
                Ok(command) => {
                    if !self.apply(command) {
                        return false;
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.drain();
                    return false;
                }
            }
        }
        self.step();
        true
    }

    /// Decode the next segment of the utterance that has waited longest for it: since the participant stopped speaking, or since its last segment was decoded. Utterances that have ended therefore take turns a segment at a time, and one that ends while another's long one is being decoded waits for at most one of its segments.
    fn step(&mut self) {
        let Some(index) = self
            .active
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| Some((index, entry.ready?)))
            .min_by_key(|&(_, ready)| ready)
            .map(|(index, _)| index)
        else {
            return;
        };
        let mut entry = self.active.remove(index).expect("index is in range");
        // The others that have ended now wait behind it, at least for this segment.
        for other in &mut self.active {
            if other.ready.is_some() && other.told != Told::Waiting {
                other.told = Told::Waiting;
                tell(&other.sink, TranscriptEvent::Progress(Progress::Waiting));
            }
        }
        match panic::catch_unwind(AssertUnwindSafe(|| self.advance(&mut entry))) {
            Ok(true) => {
                entry.ready = Some(Instant::now());
                self.active.push_back(entry);
            }
            Ok(false) => {}
            Err(panic) => {
                // `entry` is no longer among the active utterances that disabling fails, so it is failed here.
                self.disable(&panic_message(&*panic));
                self.conclude(
                    &entry,
                    TranscriptEvent::Failed(Error::Stopped(STOPPED_MESSAGE.into())),
                );
            }
        }
    }

    /// Tell `entry`'s sink its outcome, `event`, once it is no longer open, so that an utterance ending meanwhile does not wait behind it.
    fn conclude(&self, entry: &OfflineEntry, event: TranscriptEvent) {
        pending(&self.pending).remove(&entry.key);
        tell(&entry.sink, event);
    }

    /// Decode `entry`'s next segment, and report the transcript to its sink once every segment is decoded, or the failure. Returns whether it has more to decode.
    fn advance(&mut self, entry: &mut OfflineEntry) -> bool {
        if entry.samples.is_empty() {
            // Nothing was said; the transcript is empty, as the streaming path reports it.
            self.conclude(entry, TranscriptEvent::Final(String::new()));
            return false;
        }
        if entry.cuts.is_empty() {
            // Every segment is found at once, so that each can be told how many there are. The detector resets for each search, so finding them all first finds the same ones.
            let mut start = 0;
            while start < entry.samples.len() {
                let cut = self.segmenter.cut(&entry.samples, start);
                start = cut.end();
                entry.cuts.push(cut);
            }
        }
        let cut = entry.cuts[entry.segments];
        let parts = entry.cuts.len();
        // An utterance transcribed at once and whole needs no word of it: the server has said it is being transcribed.
        if parts > 1 || entry.told == Told::Waiting {
            tell(
                &entry.sink,
                TranscriptEvent::Progress(Progress::Transcribing {
                    part: entry.segments + 1,
                    parts,
                }),
            );
        }
        entry.told = Told::Transcribing;
        let end = cut.end();
        let segment = &entry.samples[entry.decoded..end];
        // A panic here is caught by `step`, which disables recognition and fails the utterance.
        let text = match self.decode(segment) {
            Ok(text) => text,
            Err(error) => {
                log::warn!(
                    "Transcription of utterance {} failed in segment {}: {error}",
                    entry.key,
                    entry.segments + 1
                );
                self.conclude(entry, TranscriptEvent::Failed(error));
                return false;
            }
        };
        entry.segments += 1;
        if matches!(cut, Cut::Hard(_)) {
            entry.hard += 1;
        }
        let text = text.trim();
        // Whisper writes a note such as "[BLANK_AUDIO]", or a word or two such as "you", for a clip without speech: a pause in a long utterance longer than the search for a pause, or a recording with nothing said in it, as when the participant pressed to talk by accident. So a note is left out, and so are a word or two the detector heard no speech for; an utterance left with nothing is reported as empty, which sends nothing. Anything longer is kept, in case the detector missed quiet speech.
        if !text.is_empty()
            && (annotation(text)
                || (text.split_whitespace().count() <= 2 && self.segmenter.silent(segment)))
        {
            log::info!(
                "Utterance {}: segment {} was heard as {text:?} with no speech in it, so it was left out.",
                entry.key,
                entry.segments
            );
            entry.skipped += 1;
        } else if !text.is_empty() {
            entry.texts.push(text.to_owned());
        }
        entry.decoded = end;
        self.complete(entry)
    }

    /// Report `entry`'s transcript if every segment has been decoded. Returns whether it has more to decode.
    fn complete(&mut self, entry: &mut OfflineEntry) -> bool {
        if entry.decoded < entry.samples.len() {
            return true;
        }
        if entry.segments > 1 {
            log::info!(
                "Utterance {}: {:.1} seconds transcribed in {} segments, {} cut without a pause and {} left out as having no speech.",
                entry.key,
                entry.samples.len() as f64 / f64::from(RATE),
                entry.segments,
                entry.hard,
                entry.skipped
            );
        }
        self.conclude(entry, TranscriptEvent::Final(entry.texts.join(" ")));
        false
    }

    fn decode(&mut self, samples: &[i16]) -> Result<String, Error> {
        // An utterance is begun only while the recognizer is loaded, and unloading it fails every utterance.
        self.recognizer
            .as_mut()
            .expect("decoded only while loaded")
            .transcribe(samples)
    }

    /// Fail every open utterance and unload the recognizer.
    fn drain(&mut self) {
        for entry in std::mem::take(&mut self.active) {
            self.conclude(
                &entry,
                TranscriptEvent::Failed(Error::Stopped("the server is shutting down".into())),
            );
        }
        self.recognizer = None;
    }

    /// Disable recognition after a panic: fail every open utterance and unload the recognizer.
    fn disable(&mut self, message: &str) {
        if !self.disabled {
            self.disabled = true;
            log::error!(
                "Speech recognition stopped after a panic ({message}); it is unavailable until the server restarts."
            );
        }
        self.drain();
    }

    /// Apply `command`. Returns whether the engine keeps running.
    fn apply(&mut self, command: Command) -> bool {
        match command {
            Command::Begin { key, sink } => {
                if self.recognizer.is_none() {
                    pending(&self.pending).remove(&key);
                    tell(
                        &sink,
                        TranscriptEvent::Failed(Error::Stopped(STOPPED_MESSAGE.into())),
                    );
                    return true;
                }
                self.active.push_back(OfflineEntry {
                    key,
                    sink,
                    samples: Vec::new(),
                    ready: None,
                    decoded: 0,
                    cuts: Vec::new(),
                    told: Told::Nothing,
                    texts: Vec::new(),
                    segments: 0,
                    hard: 0,
                    skipped: 0,
                });
            }
            Command::Audio { key, samples } => {
                let Some(index) = self
                    .active
                    .iter()
                    .position(|entry| entry.key == key && entry.ready.is_none())
                else {
                    return true;
                };
                let entry = &mut self.active[index];
                if entry.samples.len() + samples.len() > self.limit {
                    // Fail at once, while the participant is still speaking, rather than transcribe the start and drop the rest. Its later audio and end find no utterance.
                    let entry = self.active.remove(index).expect("index is in range");
                    log::warn!(
                        "Utterance {} passed {} seconds of audio, so it failed.",
                        entry.key,
                        self.limit / RATE as usize
                    );
                    self.conclude(
                        &entry,
                        TranscriptEvent::Failed(Error::TooLong(self.limit / RATE as usize)),
                    );
                    return true;
                }
                entry.samples.extend(samples);
            }
            // An utterance that failed while it was spoken, as too long, has nothing left to end.
            Command::End { key, at, waiting } => {
                if let Some(entry) = self.find(key) {
                    entry.ready = Some(at);
                    if waiting {
                        entry.told = Told::Waiting;
                    }
                }
            }
            Command::Cancel { key } => {
                self.active.retain(|entry| entry.key != key);
                pending(&self.pending).remove(&key);
            }
            Command::Stop => {
                self.drain();
                return false;
            }
        }
        true
    }

    fn find(&mut self, key: u64) -> Option<&mut OfflineEntry> {
        self.active.iter_mut().find(|entry| entry.key == key)
    }
}

/// The offline recognition service: one recognizer thread shared by all utterances.
pub struct OfflineRecognition {
    commands: Sender<Command>,
    pending: Pending,
    next: u64,
    exited: Exited,
    /// Join the decode thread when the server stops it, so a long decode does not outlive the server.
    thread: Option<JoinHandle<()>>,
}

impl OfflineRecognition {
    /// Recognize with `recognizer`, which decodes each clip once, whole. A clip longer than [`super::MAX_SEGMENT`] is split at its quietest points.
    pub fn spawn(recognizer: impl OfflineRecognizer + 'static) -> Self {
        Self::start(recognizer, None, MAX_UTTERANCE)
    }

    /// Recognize with `recognizer`, splitting a clip longer than [`super::MAX_SEGMENT`] at the pauses `voice` hears, and by loudness where it hears none.
    pub fn with_voice_activity(
        recognizer: impl OfflineRecognizer + 'static,
        voice: impl VoiceActivity + 'static,
    ) -> Self {
        Self::start(recognizer, Some(Box::new(voice)), MAX_UTTERANCE)
    }

    fn start(
        recognizer: impl OfflineRecognizer + 'static,
        voice: Option<Box<dyn VoiceActivity>>,
        limit: usize,
    ) -> Self {
        let (commands, receiver) = mpsc::channel();
        let exited = Exited::default();
        let guard = exited.guard();
        let pending = Pending::default();
        let shared = Arc::clone(&pending);
        let thread = thread::Builder::new()
            .name("speech-recognition-offline".into())
            .spawn(move || {
                // Dropped last, once the recognizer has been unloaded.
                let _guard = guard;
                OfflineEngine::new(receiver, recognizer, voice, limit, shared).run();
            })
            .expect("spawn the offline recognition thread");
        Self {
            commands,
            pending,
            next: 0,
            exited,
            thread: Some(thread),
        }
    }

    /// Begin an utterance whose audio arrives at `sample_rate`. Events are delivered to `sink` on the recognition thread, so it should not block, except [`Progress::Waiting`] as the utterance ends, which is delivered by [`OfflineUtterance::end`]; they are delivered one at a time, in order.
    pub fn begin(
        &mut self,
        sample_rate: u32,
        sink: impl FnMut(TranscriptEvent) + Send + 'static,
    ) -> OfflineUtterance {
        self.next += 1;
        let key = self.next;
        let sink: Sink = Arc::new(Mutex::new(Box::new(sink)));
        pending(&self.pending).insert(key, false);
        let _ = self.commands.send(Command::Begin {
            key,
            sink: Arc::clone(&sink),
        });
        OfflineUtterance {
            key,
            sink,
            pending: Arc::clone(&self.pending),
            commands: self.commands.clone(),
            resampler: Resampler::new(sample_rate, RATE),
            open: true,
        }
    }

    /// A handle that stops the recognition thread: every open utterance fails with [`Error::Stopped`], the recognizer is unloaded, and the thread exits.
    pub fn stopper(&self) -> Stopper {
        let commands = self.commands.clone();
        Stopper::new(
            move || {
                let _ = commands.send(Command::Stop);
            },
            self.exited.clone(),
        )
    }
}

impl Drop for OfflineRecognition {
    /// Waits for the decode thread to finish; in-progress decodes are abandoned and the recognizer is unloaded.
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Stop);
        let thread = self.thread.take().expect("joined only here");
        let _ = thread.join();
    }
}

/// One utterance being buffered for a whole-clip decode. Dropping it without calling [`OfflineUtterance::end`] cancels it.
pub struct OfflineUtterance {
    key: u64,
    sink: Sink,
    pending: Pending,
    commands: Sender<Command>,
    /// Resamples the utterance's audio to 16 kHz between frames.
    resampler: Resampler,
    open: bool,
}

impl OfflineUtterance {
    /// Append audio at the utterance's sample rate; it is buffered until the utterance ends.
    pub fn push(&mut self, samples: &[i16]) {
        // Ending or cancelling takes the utterance, so it is open while audio is pushed.
        let mut resampled = Vec::new();
        self.resampler.process(samples, &mut resampled);
        if resampled.is_empty() {
            return;
        }
        let _ = self.commands.send(Command::Audio {
            key: self.key,
            samples: resampled,
        });
    }

    /// Append the samples of a frame, adapting to its sample rate.
    ///
    /// The caller is responsible for frame order; a missing frame must fail the utterance (`design/live-audio.md`), for example by cancelling it.
    pub fn push_frame(&mut self, frame: &AudioFrame) {
        if frame.sample_rate != self.resampler.input_rate() {
            self.resampler = Resampler::new(frame.sample_rate, RATE);
        }
        self.push(&frame.samples);
    }

    /// The participant finished speaking: decode the buffered clip and deliver the final transcript. If another utterance that has ended is still being transcribed, the sink is first told, before this returns, that this one waits.
    pub fn end(mut self) {
        self.open = false;
        // Taken together, so that the utterances that end first are first in line. One that has already failed waits for nothing.
        let (at, waiting) = {
            let mut pending = pending(&self.pending);
            let ended = pending.get_mut(&self.key).is_some_and(|ended| {
                *ended = true;
                true
            });
            let others = pending
                .iter()
                .any(|(&key, &ended)| ended && key != self.key);
            (Instant::now(), ended && others)
        };
        // The sink is held while the end is sent and the wait told, so that nothing the engine tells of this utterance comes first, and the engine has the end once the wait is told.
        let mut sink = self.sink.lock().unwrap_or_else(PoisonError::into_inner);
        let _ = self.commands.send(Command::End {
            key: self.key,
            at,
            waiting,
        });
        if waiting {
            sink(TranscriptEvent::Progress(Progress::Waiting));
        }
    }

    /// Discard the utterance without a transcript. No further events are delivered.
    pub fn cancel(mut self) {
        self.open = false;
        let _ = self.commands.send(Command::Cancel { key: self.key });
    }
}

impl Drop for OfflineUtterance {
    fn drop(&mut self) {
        if self.open {
            let _ = self.commands.send(Command::Cancel { key: self.key });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stt::MAX_SEGMENT;
    use std::sync::mpsc;

    /// A fake offline recognizer that returns `text` for every clip.
    struct Fake(String);

    impl OfflineRecognizer for Fake {
        fn transcribe(&mut self, _samples: &[i16]) -> Result<String, Error> {
            Ok(self.0.clone())
        }
    }

    #[test]
    fn decodes_the_buffered_clip_when_it_ends() {
        let (sender, receiver) = mpsc::channel();
        let mut offline = OfflineRecognition::spawn(Fake("the final words".to_owned()));
        let mut handle = offline.begin(RATE, move |event| {
            let _ = sender.send(event);
        });
        // No event is delivered while the participant is speaking.
        handle.push(&vec![0; 1600]);
        assert!(receiver.try_recv().is_err());
        handle.end();
        let event = receiver.recv().unwrap();
        assert!(
            matches!(&event, TranscriptEvent::Final(text) if text == "the final words"),
            "expected Final, got {event:?}"
        );
    }

    #[test]
    fn a_cancelled_utterance_reports_nothing() {
        let mut offline = OfflineRecognition::spawn(Fake("hello".to_owned()));
        let handle = offline.begin(RATE, |event| unreachable!("reported {event:?}"));
        handle.cancel();
        // Reporting it would have disabled recognition.
        let (sender, receiver) = mpsc::channel();
        let handle = offline.begin(RATE, move |event| {
            let _ = sender.send(event);
        });
        handle.end();
        assert_eq!(
            receiver.recv().unwrap(),
            TranscriptEvent::Final(String::new())
        );
    }

    #[test]
    fn an_empty_clip_is_an_empty_transcript() {
        let (sender, receiver) = mpsc::channel();
        let mut offline = OfflineRecognition::spawn(Fake("hello".to_owned()));
        let handle = offline.begin(RATE, move |event| {
            let _ = sender.send(event);
        });
        handle.end();
        let event = receiver.recv().unwrap();
        assert!(
            matches!(&event, TranscriptEvent::Final(text) if text.is_empty()),
            "expected an empty Final, got {event:?}"
        );
    }

    #[test]
    fn stopping_fails_open_utterances() {
        let (sender, receiver) = mpsc::channel();
        let mut offline = OfflineRecognition::spawn(Fake("hello".to_owned()));
        let handle = offline.begin(RATE, move |event| {
            let _ = sender.send(event);
        });
        let stopper = offline.stopper();
        stopper.stop();
        assert!(stopper.wait(std::time::Instant::now() + std::time::Duration::from_secs(5)));
        drop(handle);
        let event = receiver.recv().unwrap();
        let TranscriptEvent::Failed(Error::Stopped(_)) = &event else {
            unreachable!("expected Failed(Stopped), got {event:?}");
        };
    }

    use std::{
        ops::Range,
        sync::{Arc, Mutex},
        time::Duration,
    };

    /// How long a test waits for an event, and a gated decode for the test to release it, so that a test whose expectation fails while a decode is held fails rather than hangs.
    const WAIT: Duration = Duration::from_secs(10);

    /// A fake offline recognizer that records the length of each clip it is given and answers with `answer`, called with the clip and its index.
    struct Recorder {
        calls: Arc<Mutex<Vec<usize>>>,
        answer: Box<Answer>,
    }

    /// Answers the clip of the given index.
    type Answer = dyn FnMut(usize, &[i16]) -> Result<String, Error> + Send;

    impl Recorder {
        fn new(
            answer: impl FnMut(usize, &[i16]) -> Result<String, Error> + Send + 'static,
        ) -> (Self, Arc<Mutex<Vec<usize>>>) {
            let calls = Arc::new(Mutex::new(Vec::new()));
            (
                Self {
                    calls: Arc::clone(&calls),
                    answer: Box::new(answer),
                },
                calls,
            )
        }
    }

    impl OfflineRecognizer for Recorder {
        fn transcribe(&mut self, samples: &[i16]) -> Result<String, Error> {
            let length = samples.len();
            assert!(length <= MAX_SEGMENT, "{length} samples");
            let index = {
                let mut calls = self.calls.lock().unwrap();
                calls.push(samples.len());
                calls.len() - 1
            };
            (self.answer)(index, samples)
        }
    }

    /// A steady signal with no pause in it, `seconds` long.
    fn steady(seconds: usize) -> Vec<i16> {
        (0..RATE as usize * seconds)
            .map(|index| if index % 40 < 20 { 6000 } else { -6000 })
            .collect()
    }

    /// The utterance's outcome, past any progress it was told.
    fn collect(receiver: &mpsc::Receiver<TranscriptEvent>) -> TranscriptEvent {
        loop {
            match receiver
                .recv_timeout(WAIT)
                .expect("the utterance's outcome is reported in time")
            {
                TranscriptEvent::Progress(_) => {}
                event => return event,
            }
        }
    }

    #[test]
    fn a_short_utterance_is_decoded_whole_once() {
        for length in [RATE as usize * 10, MAX_SEGMENT] {
            let (recorder, calls) = Recorder::new(|_, _| Ok("  Short and sweet. ".to_owned()));
            let mut offline = OfflineRecognition::spawn(recorder);
            let (sender, receiver) = mpsc::channel();
            let mut handle = offline.begin(RATE, move |event| {
                let _ = sender.send(event);
            });
            handle.push(&vec![1000; length]);
            handle.end();
            assert_eq!(
                collect(&receiver),
                TranscriptEvent::Final("Short and sweet.".to_owned())
            );
            assert_eq!(*calls.lock().unwrap(), vec![length]);
        }
    }

    #[test]
    fn a_long_utterance_is_decoded_in_segments_joined_in_order() {
        let (recorder, calls) = Recorder::new(|index, _| {
            Ok(["  The first part, ", "", "and the third.\n"][index].to_owned())
        });
        let mut offline = OfflineRecognition::spawn(recorder);
        let (sender, receiver) = mpsc::channel();
        let mut handle = offline.begin(RATE, move |event| {
            let _ = sender.send(event);
        });
        // In 100 ms frames, as the headset sends them.
        let audio = steady(70);
        for frame in audio.chunks(RATE as usize / 10) {
            handle.push(frame);
        }
        handle.end();
        assert_eq!(
            collect(&receiver),
            TranscriptEvent::Final("The first part, and the third.".to_owned())
        );
        // Without a pause, each segment but the last is as long as it can be, and together they are the whole utterance.
        assert_eq!(
            *calls.lock().unwrap(),
            vec![MAX_SEGMENT, MAX_SEGMENT, audio.len() - 2 * MAX_SEGMENT]
        );
    }

    #[test]
    fn segments_are_the_utterance_in_order() {
        // Each sample is its own index, scaled, so a segment shows where it came from.
        let audio: Vec<i16> = (0..RATE as usize * 80)
            .map(|index| (index / 1000) as i16)
            .collect();
        let starts = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&starts);
        let (recorder, calls) = Recorder::new(move |index, samples| {
            seen.lock()
                .unwrap()
                .push((samples[0], samples[samples.len() - 1]));
            Ok(format!("part {index}"))
        });
        let mut offline = OfflineRecognition::spawn(recorder);
        let (sender, receiver) = mpsc::channel();
        let mut handle = offline.begin(RATE, move |event| {
            let _ = sender.send(event);
        });
        handle.push(&audio);
        handle.end();
        let TranscriptEvent::Final(text) = collect(&receiver) else {
            unreachable!("expected a transcript");
        };
        let calls = calls.lock().unwrap().clone();
        assert_eq!(calls.iter().sum::<usize>(), audio.len());
        let parts: Vec<String> = (0..calls.len())
            .map(|index| format!("part {index}"))
            .collect();
        assert_eq!(text, parts.join(" "));
        // Each segment starts where the one before it ended.
        let mut next = 0;
        for (length, (first, last)) in calls.iter().zip(starts.lock().unwrap().iter()) {
            assert_eq!(*first, audio[next]);
            next += length;
            assert_eq!(*last, audio[next - 1]);
        }
    }

    /// A fake detector that hears speech everywhere but in `gap`, in clip indices, given audio that starts at `offset` in the clip.
    struct Gap {
        offset: usize,
        gap: Range<usize>,
    }

    impl VoiceActivity for Gap {
        fn speech(&mut self, samples: &[i16]) -> Result<Vec<Range<usize>>, Error> {
            let gap = self.gap.start - self.offset..self.gap.end - self.offset;
            Ok(vec![0..gap.start, gap.end..samples.len()])
        }
    }

    #[test]
    fn a_long_utterance_is_split_where_the_detector_hears_a_pause() {
        // Loudness alone shows no pause.
        let audio = steady(40);
        let gap = RATE as usize * 21..RATE as usize * 22;
        // The detector hears only the first segment's search; the rest fits in one segment.
        let detector = Gap {
            offset: segment::listen(&audio, 0).unwrap().start,
            gap: gap.clone(),
        };
        let (recorder, calls) = Recorder::new(|_, _| Ok("words".to_owned()));
        let mut offline = OfflineRecognition::with_voice_activity(recorder, detector);
        let (sender, receiver) = mpsc::channel();
        let mut handle = offline.begin(RATE, move |event| {
            let _ = sender.send(event);
        });
        handle.push(&audio);
        handle.end();
        assert_eq!(
            collect(&receiver),
            TranscriptEvent::Final("words words".to_owned())
        );
        let first = calls.lock().unwrap()[0];
        assert!(gap.contains(&first), "cut at {first}");
    }

    struct Panicking;

    impl VoiceActivity for Panicking {
        fn speech(&mut self, _samples: &[i16]) -> Result<Vec<Range<usize>>, Error> {
            panic!("the detector broke");
        }
    }

    #[test]
    fn a_panic_in_the_detector_falls_back_to_loudness() {
        log::set_max_level(log::LevelFilter::Trace);
        let (recorder, calls) = Recorder::new(|_, _| Ok("words".to_owned()));
        let mut offline = OfflineRecognition::with_voice_activity(recorder, Panicking);
        for _ in 0..2 {
            let (sender, receiver) = mpsc::channel();
            let mut handle = offline.begin(RATE, move |event| {
                let _ = sender.send(event);
            });
            handle.push(&steady(40));
            handle.end();
            assert_eq!(
                collect(&receiver),
                TranscriptEvent::Final("words words".to_owned())
            );
        }
        assert_eq!(calls.lock().unwrap().len(), 4);
    }

    #[test]
    fn an_utterance_longer_than_the_limit_fails_as_it_passes_it() {
        log::set_max_level(log::LevelFilter::Trace);
        let (recorder, calls) = Recorder::new(|_, _| unreachable!("nothing is decoded"));
        let mut offline = OfflineRecognition::start(recorder, None, RATE as usize * 2);
        let (sender, receiver) = mpsc::channel();
        let mut handle = offline.begin(RATE, move |event| {
            let _ = sender.send(event);
        });
        // Exactly the limit is accepted.
        handle.push(&vec![1000; RATE as usize * 2]);
        assert!(receiver.recv_timeout(Duration::from_millis(200)).is_err());
        // One more frame fails it at once, before it ends.
        handle.push(&vec![1000; RATE as usize / 10]);
        assert_eq!(
            collect(&receiver),
            TranscriptEvent::Failed(Error::TooLong(2))
        );
        handle.push(&vec![1000; RATE as usize / 10]);
        handle.end();
        assert!(receiver.recv_timeout(Duration::from_millis(200)).is_err());
        assert!(calls.lock().unwrap().is_empty());
        assert_eq!(
            Error::TooLong(MAX_UTTERANCE / RATE as usize).to_string(),
            "the speech was longer than 5 minutes, the most that is transcribed at once"
        );
    }

    #[test]
    fn a_failed_segment_fails_the_utterance() {
        let (recorder, calls) = Recorder::new(|index, _| match index {
            1 => Err(Error::Engine("the decoder broke".to_owned())),
            _ => Ok("words".to_owned()),
        });
        let mut offline = OfflineRecognition::spawn(recorder);
        let (sender, receiver) = mpsc::channel();
        let mut handle = offline.begin(RATE, move |event| {
            let _ = sender.send(event);
        });
        handle.push(&steady(70));
        handle.end();
        assert_eq!(
            collect(&receiver),
            TranscriptEvent::Failed(Error::Engine("the decoder broke".to_owned()))
        );
        assert!(receiver.recv_timeout(Duration::from_millis(200)).is_err());
        // The rest of the utterance is not decoded.
        assert_eq!(calls.lock().unwrap().len(), 2);
    }

    #[test]
    fn an_utterance_cancelled_between_segments_reports_nothing() {
        let (started, decoding) = mpsc::channel();
        let (release, gate) = mpsc::channel::<()>();
        let gate = Mutex::new(gate);
        let (recorder, calls) = Recorder::new(move |_, _| {
            let _ = started.send(());
            let _ = gate.lock().unwrap().recv_timeout(WAIT);
            Ok("words".to_owned())
        });
        let mut offline = OfflineRecognition::spawn(recorder);
        // Declared after `offline`, so a failed assertion's unwinding drops it first, letting a held decode go on rather than hanging as `offline` joins the engine's thread.
        let release = release;
        // Reporting an outcome would panic on the engine's thread, which disables recognition, as the next utterance would show.
        let mut handle = offline.begin(RATE, |event| {
            let TranscriptEvent::Progress(_) = event else {
                unreachable!("reported {event:?}");
            };
        });
        handle.push(&steady(70));
        // The engine's commands are applied in order, so cancelling needs the key, which only the handle knows.
        let key = handle.key;
        let commands = handle.commands.clone();
        handle.end();
        decoding
            .recv_timeout(WAIT)
            .expect("a decode starts in time");
        commands.send(Command::Cancel { key }).unwrap();
        release.send(()).unwrap();
        drop(release);
        let (sender, receiver) = mpsc::channel();
        let handle = offline.begin(RATE, move |event| {
            let _ = sender.send(event);
        });
        handle.end();
        assert_eq!(collect(&receiver), TranscriptEvent::Final(String::new()));
        assert_eq!(calls.lock().unwrap().len(), 1);
    }

    #[test]
    fn an_utterance_that_ends_goes_before_the_next_segment_of_a_long_one() {
        let (started, decoding) = mpsc::channel();
        let (release, gate) = mpsc::channel::<()>();
        let gate = Mutex::new(gate);
        let (recorder, calls) = Recorder::new(move |_, samples| {
            let _ = started.send(());
            let _ = gate.lock().unwrap().recv_timeout(WAIT);
            Ok(format!("{}", samples.len()))
        });
        let mut offline = OfflineRecognition::spawn(recorder);
        // Declared after `offline`, so a failed assertion's unwinding drops it first, letting a held decode go on rather than hanging as `offline` joins the engine's thread.
        let release = release;
        let (sender, receiver) = mpsc::channel();
        let long_sender = sender.clone();
        let mut long = offline.begin(RATE, move |event| {
            let _ = long_sender.send(("long", event));
        });
        long.push(&steady(70));
        long.end();
        // While the long utterance's first segment is decoded, a short one ends.
        decoding
            .recv_timeout(WAIT)
            .expect("a decode starts in time");
        let mut short = offline.begin(RATE, move |event| {
            let _ = sender.send(("short", event));
        });
        short.push(&vec![1000; RATE as usize]);
        short.end();
        for _ in 0..4 {
            release.send(()).unwrap();
        }
        // The short one, told it waits as it ends, is transcribed next, whole; the long one waits for it, and is then transcribed by parts.
        let waiting = TranscriptEvent::Progress(Progress::Waiting);
        let part = |part, parts| TranscriptEvent::Progress(Progress::Transcribing { part, parts });
        let events: Vec<_> = (0..8)
            .map(|_| {
                receiver
                    .recv_timeout(WAIT)
                    .expect("an event is reported in time")
            })
            .collect();
        assert_eq!(
            events,
            [
                ("long", part(1, 3)),
                ("short", waiting.clone()),
                ("long", waiting),
                ("short", part(1, 1)),
                ("short", TranscriptEvent::Final(format!("{}", RATE))),
                ("long", part(2, 3)),
                ("long", part(3, 3)),
                (
                    "long",
                    TranscriptEvent::Final(format!(
                        "{MAX_SEGMENT} {MAX_SEGMENT} {}",
                        RATE as usize * 70 - 2 * MAX_SEGMENT
                    ))
                ),
            ]
        );
        assert_eq!(
            *calls.lock().unwrap(),
            vec![
                MAX_SEGMENT,
                RATE as usize,
                MAX_SEGMENT,
                RATE as usize * 70 - 2 * MAX_SEGMENT
            ]
        );
    }

    #[test]
    fn an_utterance_is_told_each_of_its_parts_and_nothing_for_one() {
        let part = |part, parts| TranscriptEvent::Progress(Progress::Transcribing { part, parts });
        for (seconds, told) in [(10, vec![]), (70, vec![part(1, 3), part(2, 3), part(3, 3)])] {
            let mut offline = OfflineRecognition::spawn(Fake("words".to_owned()));
            let (sender, receiver) = mpsc::channel();
            let mut handle = offline.begin(RATE, move |event| {
                let _ = sender.send(event);
            });
            handle.push(&steady(seconds));
            handle.end();
            let mut events: Vec<_> = receiver.iter().take(told.len() + 1).collect();
            let Some(TranscriptEvent::Final(_)) = events.pop() else {
                unreachable!("the outcome comes last");
            };
            assert_eq!(events, told);
        }
    }

    #[test]
    fn an_utterance_waits_only_behind_one_still_being_transcribed() {
        let (started, decoding) = mpsc::channel();
        let (release, gate) = mpsc::channel::<()>();
        let gate = Mutex::new(gate);
        let (recorder, _) = Recorder::new(move |_, _| {
            let _ = started.send(());
            let _ = gate.lock().unwrap().recv_timeout(WAIT);
            Ok("words".to_owned())
        });
        let mut offline = OfflineRecognition::start(recorder, None, RATE as usize * 2);
        // Declared after `offline`, so a failed assertion's unwinding drops it first, letting a held decode go on rather than hanging as `offline` joins the engine's thread.
        let release = release;
        let listen = |offline: &mut OfflineRecognition| {
            let (sender, receiver) = mpsc::channel();
            let handle = offline.begin(RATE, move |event| {
                let _ = sender.send(event);
            });
            (handle, receiver)
        };
        let (mut first, _first) = listen(&mut offline);
        first.push(&[1000; 1600]);
        first.end();
        decoding
            .recv_timeout(WAIT)
            .expect("the first utterance's decode starts");
        // One that ends while another is transcribed is told at once, before its end returns, that it waits.
        let (mut second, waiting) = listen(&mut offline);
        second.push(&[1000; 1600]);
        second.end();
        assert_eq!(
            waiting.try_recv().ok(),
            Some(TranscriptEvent::Progress(Progress::Waiting)),
            "the second utterance is told it waits as it ends"
        );
        release.send(()).unwrap();
        decoding
            .recv_timeout(WAIT)
            .expect("the second utterance's decode starts once the first's finishes");
        // Its turn comes, and it says so, after the one it waited for is told its outcome.
        assert_eq!(
            waiting.recv_timeout(WAIT).ok(),
            Some(TranscriptEvent::Progress(Progress::Transcribing {
                part: 1,
                parts: 1
            })),
            "the second utterance is told it is transcribed once its turn comes"
        );
        release.send(()).unwrap();
        assert_eq!(
            collect(&waiting),
            TranscriptEvent::Final("words".to_owned())
        );
        // One that failed as too long, ending after its failure, makes nothing wait.
        let (mut long, failed) = listen(&mut offline);
        long.push(&[1000; RATE as usize * 3]);
        assert_eq!(collect(&failed), TranscriptEvent::Failed(Error::TooLong(2)));
        long.end();
        let (mut last, alone) = listen(&mut offline);
        last.push(&[1000; 1600]);
        last.end();
        decoding
            .recv_timeout(WAIT)
            .expect("the last utterance's decode starts");
        release.send(()).unwrap();
        assert_eq!(
            alone.recv_timeout(WAIT).ok(),
            Some(TranscriptEvent::Final("words".to_owned())),
            "the last utterance, behind only one that failed, is told nothing before its outcome"
        );
    }

    #[test]
    fn long_utterances_take_turns_a_segment_at_a_time() {
        let (started, decoding) = mpsc::channel();
        let (release, gate) = mpsc::channel::<()>();
        let gate = Mutex::new(gate);
        let (recorder, _) = Recorder::new(move |_, samples| {
            let _ = started.send(samples[0]);
            let _ = gate.lock().unwrap().recv_timeout(WAIT);
            Ok(String::new())
        });
        let mut offline = OfflineRecognition::spawn(recorder);
        // Declared after `offline`, so a failed assertion's unwinding drops it first, letting a held decode go on rather than hanging as `offline` joins the engine's thread.
        let release = release;
        // Each utterance's audio is its own value, so a segment shows whose it is.
        let mut first = offline.begin(RATE, |_| {});
        first.push(&vec![1000; RATE as usize * 70]);
        first.end();
        assert_eq!(
            decoding
                .recv_timeout(WAIT)
                .expect("a decode starts in time"),
            1000
        );
        let mut second = offline.begin(RATE, |_| {});
        second.push(&vec![2000; RATE as usize * 70]);
        second.end();
        let mut order = vec![1000];
        for _ in 0..5 {
            release.send(()).unwrap();
            order.push(
                decoding
                    .recv_timeout(WAIT)
                    .expect("a decode starts in time"),
            );
        }
        release.send(()).unwrap();
        assert_eq!(order, vec![1000, 2000, 1000, 2000, 1000, 2000]);
    }

    #[test]
    fn short_utterances_do_not_hold_up_a_long_one() {
        let (started, decoding) = mpsc::channel();
        let (release, gate) = mpsc::channel::<()>();
        let gate = Mutex::new(gate);
        let (recorder, _) = Recorder::new(move |_, samples| {
            let _ = started.send(samples[0]);
            let _ = gate.lock().unwrap().recv_timeout(WAIT);
            Ok(String::new())
        });
        let mut offline = OfflineRecognition::spawn(recorder);
        // Declared after `offline`, so a failed assertion's unwinding drops it first, letting a held decode go on rather than hanging as `offline` joins the engine's thread.
        let release = release;
        let mut long = offline.begin(RATE, |_| {});
        long.push(&vec![1000; RATE as usize * 40]);
        long.end();
        let mut order = vec![
            decoding
                .recv_timeout(WAIT)
                .expect("a decode starts in time"),
        ];
        // Each short utterance ends while the previous segment is being decoded.
        for value in [2000, 3000] {
            let mut short = offline.begin(RATE, |_| {});
            short.push(&vec![value; RATE as usize]);
            short.end();
            release.send(()).unwrap();
            order.push(
                decoding
                    .recv_timeout(WAIT)
                    .expect("a decode starts in time"),
            );
        }
        release.send(()).unwrap();
        order.push(
            decoding
                .recv_timeout(WAIT)
                .expect("a decode starts in time"),
        );
        release.send(()).unwrap();
        // The first short one ended before the long one's first segment was decoded, so it goes first; the second ended after, so it waits.
        assert_eq!(order, vec![1000, 2000, 1000, 3000]);
    }

    /// A fake detector that hears speech in any audio that is not silent.
    struct Loud;

    impl VoiceActivity for Loud {
        fn speech(&mut self, samples: &[i16]) -> Result<Vec<Range<usize>>, Error> {
            let mut speech = Vec::new();
            if samples.iter().any(|&sample| sample != 0) {
                speech.push(0..samples.len());
            }
            Ok(speech)
        }
    }

    #[test]
    fn a_word_or_two_for_a_segment_without_speech_in_a_long_utterance_is_left_out() {
        for (heard, expected) in [
            ("you", "Speech. Speech."),
            ("Thank you.", "Speech. Speech."),
            (
                "Something said very quietly.",
                "Speech. Something said very quietly. Speech.",
            ),
        ] {
            let (recorder, calls) = Recorder::new(move |_, samples| {
                Ok(if samples.iter().all(|&sample| sample == 0) {
                    heard.to_owned()
                } else {
                    "Speech.".to_owned()
                })
            });
            let mut offline = OfflineRecognition::with_voice_activity(recorder, Loud);
            let (sender, receiver) = mpsc::channel();
            let mut handle = offline.begin(RATE, move |event| {
                let _ = sender.send(event);
            });
            // Speech, a pause longer than a segment, and speech again.
            handle.push(&steady(20));
            handle.push(&vec![0; RATE as usize * 30]);
            handle.push(&steady(20));
            handle.end();
            assert_eq!(
                collect(&receiver),
                TranscriptEvent::Final(expected.to_owned())
            );
            // Every segment is decoded, and together they are the whole utterance.
            let calls = calls.lock().unwrap().clone();
            assert_eq!(calls.len(), 3, "{calls:?}");
            assert_eq!(calls.iter().sum::<usize>(), RATE as usize * 70);
        }
    }

    #[test]
    fn without_a_detector_a_word_for_a_pause_is_kept() {
        let (recorder, _) = Recorder::new(|_, samples| {
            Ok(if samples.iter().all(|&sample| sample == 0) {
                "you".to_owned()
            } else {
                "Speech.".to_owned()
            })
        });
        let mut offline = OfflineRecognition::spawn(recorder);
        let (sender, receiver) = mpsc::channel();
        let mut handle = offline.begin(RATE, move |event| {
            let _ = sender.send(event);
        });
        handle.push(&steady(20));
        handle.push(&vec![0; RATE as usize * 30]);
        handle.push(&steady(20));
        handle.end();
        assert_eq!(
            collect(&receiver),
            TranscriptEvent::Final("Speech. you Speech.".to_owned())
        );
    }

    /// The transcript of a short utterance of `audio` that `recognizer` hears as `heard`, with `voice` to hear speech in it, if any, and how many times it was decoded.
    fn short(
        heard: &'static str,
        voice: Option<Box<dyn VoiceActivity>>,
        audio: &[i16],
    ) -> (TranscriptEvent, usize) {
        let (recorder, calls) = Recorder::new(move |_, _| Ok(heard.to_owned()));
        let mut offline = OfflineRecognition::start(recorder, voice, MAX_UTTERANCE);
        let (sender, receiver) = mpsc::channel();
        let mut handle = offline.begin(RATE, move |event| {
            let _ = sender.send(event);
        });
        handle.push(audio);
        handle.end();
        let event = collect(&receiver);
        let calls = calls.lock().unwrap().len();
        (event, calls)
    }

    #[test]
    fn a_short_utterance_with_no_speech_in_it_is_empty() {
        log::set_max_level(log::LevelFilter::Trace);
        let silence = vec![0; RATE as usize * 2];
        let empty = (TranscriptEvent::Final(String::new()), 1);
        // A note of noise is left out, with a detector or without one.
        for heard in [
            "[BLANK_AUDIO]",
            " (static) ",
            "[ Silence ]",
            "[Music] (static)",
        ] {
            assert_eq!(short(heard, None, &silence), empty, "{heard}");
            assert_eq!(short(heard, Some(Box::new(Loud)), &silence), empty);
        }
        // A word or two is left out where the detector hears no speech.
        for heard in ["you", "Thank you."] {
            assert_eq!(short(heard, Some(Box::new(Loud)), &silence), empty);
        }
    }

    #[test]
    fn a_short_utterance_keeps_words_that_may_be_speech() {
        let kept = |text: &str| (TranscriptEvent::Final(text.to_owned()), 1);
        // Words the detector heard speech for.
        assert_eq!(
            short("Yes.", Some(Box::new(Loud)), &steady(1)),
            kept("Yes.")
        );
        // A word or two without a detector to say there was no speech.
        assert_eq!(short("Yes.", None, &vec![0; RATE as usize]), kept("Yes."));
        // Anything longer, in case the detector missed quiet speech.
        assert_eq!(
            short(
                "Something said very quietly.",
                Some(Box::new(Loud)),
                &vec![0; RATE as usize]
            ),
            kept("Something said very quietly.")
        );
        // A note among words.
        assert_eq!(
            short(
                "(laughs) Yes, do it.",
                Some(Box::new(Loud)),
                &vec![0; RATE as usize]
            ),
            kept("(laughs) Yes, do it.")
        );
    }

    #[test]
    fn a_note_of_noise_in_a_long_utterance_is_left_out() {
        let (recorder, _) = Recorder::new(|index, _| {
            Ok(["Hello.", "[BLANK_AUDIO]", " (water running) "][index].to_owned())
        });
        let mut offline = OfflineRecognition::spawn(recorder);
        let (sender, receiver) = mpsc::channel();
        let mut handle = offline.begin(RATE, move |event| {
            let _ = sender.send(event);
        });
        handle.push(&steady(70));
        handle.end();
        assert_eq!(
            collect(&receiver),
            TranscriptEvent::Final("Hello.".to_owned())
        );
        assert!(annotation("[BLANK_AUDIO]") && annotation("(buzzing)"));
        assert!(!annotation("(a) and (b)") && !annotation("[1] is cited") && !annotation("()x"));
        assert!(!annotation("") && !annotation("(") && !annotation("words"));
        assert!(annotation("[Music] (static)") && annotation("()") && !annotation("(a]"));
        assert!(!annotation("[BLANK_AUDIO] yes") && !annotation("[a [b]]"));
    }

    struct Crashing;

    impl OfflineRecognizer for Crashing {
        fn transcribe(&mut self, samples: &[i16]) -> Result<String, Error> {
            if samples.len() < MAX_SEGMENT {
                panic!("the recognizer crashed");
            }
            Ok("words".to_owned())
        }
    }

    #[test]
    fn a_panic_in_a_later_segment_disables_recognition() {
        let mut offline = OfflineRecognition::spawn(Crashing);
        let (sender, receiver) = mpsc::channel();
        let mut handle = offline.begin(RATE, move |event| {
            let _ = sender.send(event);
        });
        handle.push(&steady(40));
        handle.end();
        let TranscriptEvent::Failed(Error::Stopped(_)) = collect(&receiver) else {
            unreachable!("expected TranscriptEvent::Failed(Error::Stopped(_))");
        };
        let (sender, receiver) = mpsc::channel();
        let mut handle = offline.begin(RATE, move |event| {
            let _ = sender.send(event);
        });
        handle.push(&steady(1));
        handle.end();
        let TranscriptEvent::Failed(Error::Stopped(_)) = collect(&receiver) else {
            unreachable!("expected TranscriptEvent::Failed(Error::Stopped(_))");
        };
    }

    /// Transcribes a minute of LibriSpeech, from the clips that come with the Whisper model, with Whisper and the Silero detector, as the offline backend does. Skipped without both models; see [`crate::test_models`].
    #[test]
    fn transcribes_a_long_utterance_with_whisper() {
        use crate::{
            stt::{SileroVad, SileroVadConfig, WhisperConfig, WhisperRecognizer},
            test_models,
        };

        /// Whisper, recording the length of each segment it decodes.
        struct Whisper(WhisperRecognizer, Arc<Mutex<Vec<usize>>>);
        impl OfflineRecognizer for Whisper {
            fn transcribe(&mut self, samples: &[i16]) -> Result<String, Error> {
                self.1.lock().unwrap().push(samples.len());
                self.0.transcribe(samples)
            }
        }

        let root = test_models::root();
        let config = WhisperConfig::named(&root, test_models::WHISPER_MODEL);
        let vad = SileroVadConfig::from_root(&root);
        test_models::with(
            "the Whisper and Silero voice activity models",
            &format!("bash scripts/fetch-asr-model.sh whisper {}", config.name),
            &[&config.model, &vad.model],
            || {
                let directory = config.model.join("test_wavs");
                let transcripts = std::fs::read_to_string(directory.join("trans.txt")).unwrap();
                let mut audio = Vec::new();
                let mut reference = Vec::new();
                // Each clip twice, with a short breath between clips, so the minute has pauses but none long.
                for _ in 0..2 {
                    for line in transcripts.lines() {
                        let (file, text) = line.split_once(' ').unwrap();
                        let wave = sherpa_onnx::Wave::read(directory.join(file).to_str().unwrap())
                            .unwrap();
                        let samples: Vec<i16> = wave
                            .samples()
                            .iter()
                            .map(|sample| (sample.clamp(-1.0, 1.0) * 32767.0) as i16)
                            .collect();
                        let mut resampled = Vec::new();
                        Resampler::new(wave.sample_rate() as u32, RATE)
                            .process(&samples, &mut resampled);
                        audio.extend(resampled);
                        audio.extend(std::iter::repeat_n(0, RATE as usize * 4 / 10));
                        reference.extend(text.split_whitespace().map(str::to_lowercase));
                    }
                }
                let length = audio.len();
                assert!(length > 2 * MAX_SEGMENT, "{length} samples");

                // With the detector, as the server runs, and by loudness alone, as when its model is not installed.
                for detector in [true, false] {
                    let whisper = WhisperRecognizer::new(&config).unwrap();
                    let segments = Arc::new(Mutex::new(Vec::new()));
                    let whisper = Whisper(whisper, Arc::clone(&segments));
                    let mut offline = if detector {
                        let vad = SileroVad::new(&vad).unwrap();
                        OfflineRecognition::with_voice_activity(whisper, vad)
                    } else {
                        OfflineRecognition::spawn(whisper)
                    };
                    let (sender, receiver) = mpsc::channel();
                    let mut handle = offline.begin(RATE, move |event| {
                        let _ = sender.send(event);
                    });
                    let started = std::time::Instant::now();
                    handle.push(&audio);
                    handle.end();
                    let mut told = Vec::new();
                    let event = loop {
                        match receiver.recv_timeout(Duration::from_secs(600)).unwrap() {
                            TranscriptEvent::Progress(progress) => told.push(progress),
                            event => break event,
                        }
                    };
                    let TranscriptEvent::Final(text) = event else {
                        unreachable!("{event:?}");
                    };
                    // Each part was told as it started, counted against all of them.
                    let parts = segments.lock().unwrap().len();
                    let expected: Vec<_> = (1..=parts)
                        .map(|part| Progress::Transcribing { part, parts })
                        .collect();
                    assert_eq!(told, expected);
                    let words: Vec<String> = text
                        .to_lowercase()
                        .split(|character: char| {
                            !(character.is_alphanumeric() || character == '\'')
                        })
                        .filter(|word| !word.is_empty())
                        .map(str::to_owned)
                        .collect();
                    let errors = edit_distance(&reference, &words);
                    let segments: Vec<String> = segments
                        .lock()
                        .unwrap()
                        .iter()
                        .map(|length| format!("{:.2}", *length as f64 / f64::from(RATE)))
                        .collect();
                    eprintln!(
                        "{} {:.1} seconds transcribed in {:.1} seconds, in segments of {} seconds, {errors} word errors in {} words: {text}",
                        if detector { "Silero:" } else { "Loudness:" },
                        audio.len() as f64 / f64::from(RATE),
                        started.elapsed().as_secs_f64(),
                        segments.join(", "),
                        reference.len()
                    );
                    // A cut through a word would cost at least a word on each side of it; LibriSpeech spells "dishonoured" and "for ever" in British style, which Whisper does not, so a perfect transcript has 6 errors.
                    assert!(errors * 20 <= reference.len(), "{errors} errors: {text}");
                }
            },
        );
    }

    /// With Whisper and the Silero detector, as the offline backend runs: what Whisper writes for a recording with nothing said in it is left out, and a short answer the detector hears as speech is kept. Skipped without both models; see [`crate::test_models`].
    #[test]
    fn a_silent_recording_is_empty_with_whisper_and_silero() {
        use crate::{
            stt::{SileroVad, SileroVadConfig, WhisperConfig, WhisperRecognizer},
            test_models,
        };

        let root = test_models::root();
        let config = WhisperConfig::named(&root, test_models::WHISPER_MODEL);
        let vad = SileroVadConfig::from_root(&root);
        test_models::with(
            "the Whisper and Silero voice activity models",
            &format!("bash scripts/fetch-asr-model.sh whisper {}", config.name),
            &[&config.model, &vad.model],
            || {
                // Two seconds of silence and of faint noise, as from a press to talk by accident.
                let silence = vec![0; RATE as usize * 2];
                let noise: Vec<i16> = (0..RATE as usize * 2)
                    .map(|index| ((index * 7919 % 401) as i16 - 200) / 4)
                    .collect();
                for audio in [silence, noise] {
                    let mut whisper = WhisperRecognizer::new(&config).unwrap();
                    let heard = Arc::new(Mutex::new(Vec::new()));
                    let shared = Arc::clone(&heard);
                    let (recorder, _) = Recorder::new(move |_, samples| {
                        whisper
                            .transcribe(samples)
                            .inspect(|text| shared.lock().unwrap().push(text.clone()))
                    });
                    let vad = SileroVad::new(&vad).unwrap();
                    let mut offline = OfflineRecognition::with_voice_activity(recorder, vad);
                    let (sender, receiver) = mpsc::channel();
                    let mut handle = offline.begin(RATE, move |event| {
                        let _ = sender.send(event);
                    });
                    handle.push(&audio);
                    handle.end();
                    let event = collect(&receiver);
                    eprintln!("Whisper heard {:?}", heard.lock().unwrap());
                    assert_eq!(event, TranscriptEvent::Final(String::new()));
                }
                // A word of speech the detector hears is kept, even as short as "yes": the start of a LibriSpeech clip, which Whisper would transcribe as more.
                let wave =
                    sherpa_onnx::Wave::read(config.model.join("test_wavs/0.wav").to_str().unwrap())
                        .unwrap();
                let speech: Vec<i16> = wave.samples()[..RATE as usize * 2]
                    .iter()
                    .map(|sample| (sample.clamp(-1.0, 1.0) * 32767.0) as i16)
                    .collect();
                let voice: Box<dyn VoiceActivity> = Box::new(SileroVad::new(&vad).unwrap());
                assert_eq!(
                    short("Yes.", Some(voice), &speech),
                    (TranscriptEvent::Final("Yes.".to_owned()), 1)
                );
            },
        );
    }

    /// The number of words inserted, deleted or replaced to turn `reference` into `words`.
    fn edit_distance(reference: &[String], words: &[String]) -> usize {
        let mut previous: Vec<usize> = (0..=words.len()).collect();
        for (row, expected) in reference.iter().enumerate() {
            let mut current = vec![row + 1];
            for (column, word) in words.iter().enumerate() {
                let replace = previous[column] + usize::from(expected != word);
                current.push(
                    replace
                        .min(previous[column + 1] + 1)
                        .min(current[column] + 1),
                );
            }
            previous = current;
        }
        previous[words.len()]
    }

    /// A detector that cannot hear.
    struct Deaf;

    impl VoiceActivity for Deaf {
        fn speech(&mut self, _samples: &[i16]) -> Result<Vec<Range<usize>>, Error> {
            Err(Error::Engine("the detector is deaf".into()))
        }
    }

    /// A recognizer that panics.
    struct Broken;

    impl OfflineRecognizer for Broken {
        fn transcribe(&mut self, _samples: &[i16]) -> Result<String, Error> {
            panic!("the recognizer broke")
        }
    }

    #[test]
    fn a_detector_that_fails_falls_back_to_loudness_and_a_boxed_recognizer_transcribes() {
        log::set_max_level(log::LevelFilter::Trace);
        let mut boxed: Box<dyn OfflineRecognizer> = Box::new(Fake("hello".to_owned()));
        assert_eq!(boxed.transcribe(&[]).unwrap(), "hello");
        let (recorder, calls) = Recorder::new(|_, _| Ok("words".to_owned()));
        let mut offline = OfflineRecognition::with_voice_activity(recorder, Deaf);
        let (sender, receiver) = mpsc::channel();
        let mut handle = offline.begin(RATE, move |event| {
            let _ = sender.send(event);
        });
        handle.push(&steady(40));
        handle.end();
        assert_eq!(
            collect(&receiver),
            TranscriptEvent::Final("words words".to_owned())
        );
        assert_eq!(calls.lock().unwrap().len(), 2);
    }

    #[test]
    fn a_recognizer_that_panics_disables_recognition_and_a_report_that_panics_is_survived() {
        log::set_max_level(log::LevelFilter::Trace);
        let mut offline = OfflineRecognition::spawn(Broken);
        let (sender, receiver) = mpsc::channel();
        let mut handle = offline.begin(RATE, move |event| {
            let _ = sender.send(event);
        });
        handle.push(&[1000; 160]);
        handle.end();
        let TranscriptEvent::Failed(Error::Stopped(_)) = collect(&receiver) else {
            unreachable!("a panic stops recognition");
        };
        // Told it has stopped, a sink that panics panics the engine again.
        let handle = offline.begin(RATE, |_| panic!("the sink broke"));
        handle.end();
        let (sender, receiver) = mpsc::channel();
        let handle = offline.begin(RATE, move |event| {
            let _ = sender.send(event);
        });
        handle.end();
        let TranscriptEvent::Failed(Error::Stopped(_)) = collect(&receiver) else {
            unreachable!("recognition stays stopped");
        };
    }

    /// An engine of `recognizer`, with what sends it commands.
    fn engine_of(recognizer: Fake) -> (OfflineEngine<Fake>, Sender<Command>) {
        let (sender, receiver) = mpsc::channel();
        (
            OfflineEngine::new(
                receiver,
                recognizer,
                None,
                MAX_UTTERANCE,
                Pending::default(),
            ),
            sender,
        )
    }

    fn begin(sender: &Sender<Command>, key: u64) -> mpsc::Receiver<TranscriptEvent> {
        let (events, receiver) = mpsc::channel();
        sender
            .send(Command::Begin {
                key,
                sink: Arc::new(Mutex::new(Box::new(move |event| {
                    let _ = events.send(event);
                }))),
            })
            .unwrap();
        receiver
    }

    #[test]
    fn an_engine_whose_commands_end_fails_what_is_open() {
        // While it waits for the next.
        let (mut engine, sender) = engine_of(Fake("hello".to_owned()));
        let events = begin(&sender, 1);
        assert!(engine.turn());
        drop(sender);
        assert!(!engine.turn());
        let TranscriptEvent::Failed(Error::Stopped(_)) = events.recv().unwrap() else {
            unreachable!("the open utterance fails");
        };
        // While it applies those waiting.
        let (mut engine, sender) = engine_of(Fake("hello".to_owned()));
        let events = begin(&sender, 1);
        sender
            .send(Command::Audio {
                key: 1,
                samples: vec![1000; 160],
            })
            .unwrap();
        drop(sender);
        assert!(!engine.turn());
        let TranscriptEvent::Failed(Error::Stopped(_)) = events.recv().unwrap() else {
            unreachable!("the open utterance fails");
        };
    }

    #[test]
    fn audio_at_another_rate_is_resampled_and_too_little_to_resample_is_kept() {
        let mut offline = OfflineRecognition::spawn(Fake("hello".to_owned()));
        let mut handle = offline.begin(RATE, drop::<TranscriptEvent>);
        handle.push_frame(&AudioFrame {
            gate: tomori_protocol::Gate::Agent,
            source: 0,
            sequence: 0,
            timestamp: 0,
            sample_rate: 48_000,
            samples: vec![0; 1],
        });
        handle.cancel();
    }
}
