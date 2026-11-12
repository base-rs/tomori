// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Whole-clip speech recognition for push-to-talk utterances.
//!
//! The [`OfflineRecognizer`] decodes a complete clip, once, so it is more accurate than the streaming [`super::StreamingRecognizer`]. The cost is latency: the final transcript arrives once the last of the clip is decoded, a couple of seconds after the participant lets go, instead of almost at once.
//!
//! [`OfflineRecognition`] owns one recognizer on a dedicated thread. Each utterance is driven through an [`OfflineUtterance`]: audio is buffered and resampled to 16 kHz, and the utterance is reported through its sink as exactly one [`TranscriptEvent::Final`] or [`TranscriptEvent::Failed`], once it has ended or as soon as it fails.
//!
//! The recognizer decodes at most [`super::MAX_SEGMENT`] at once, so a longer clip is split at its pauses into segments that fit, which are decoded in order and their transcripts joined; see [`super::segment`]. Each segment's end depends only on the audio up to a little past its limit, so a segment is cut and decoded as soon as that audio has arrived, while the participant is still speaking, exactly where it would be cut once the clip had ended; only the rest is decoded after they stop. The transcript so far is reported, as a [`TranscriptEvent::Partial`] of committed words, after each segment but the last. What Whisper writes for a segment without speech, bracketed notes or a word or two the voice activity detector heard no speech for, is left out, so a clip with no speech in it at all is reported as an empty transcript. Utterances that have ended take turns, a segment at a time, so an utterance that ends while a long one is being decoded waits for at most one of its segments, and they go before segments of utterances still being spoken, so decoding ahead never holds up an utterance that has ended by more than the segment already being decoded. An utterance longer than [`MAX_UTTERANCE`] fails with [`Error::TooLong`] as soon as it passes the limit.
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

/// What the decode thread and the handles share, so that one ending can tell whether it waits behind another, and the decode thread whether an utterance's end or cancellation is on its way.
#[derive(Default)]
struct Shared {
    /// Every utterance begun, not cancelled and without an outcome yet, and whether it has ended.
    open: HashMap<u64, bool>,
    /// The utterance still being spoken whose segment is being decoded ahead, if any.
    decoding: Option<u64>,
}

type Pending = Arc<Mutex<Shared>>;

fn pending(pending: &Pending) -> std::sync::MutexGuard<'_, Shared> {
    pending.lock().unwrap_or_else(PoisonError::into_inner)
}

/// An offline speech recognition engine, which decodes a complete clip, once. It cannot see a clip that is still being spoken, so while the participant speaks it is given only segments whose ends are decided.
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
    /// More of the utterance's audio, pushed `at`.
    Audio {
        key: u64,
        samples: Vec<i16>,
        at: Instant,
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
    /// When the participant stopped speaking, or when the utterance's last segment was decoded, whichever is later: it waits for its next segment from then. `None` while the participant is still speaking.
    ready: Option<Instant>,
    /// While the participant is still speaking, since when the audio has decided where its next segment ends, so that it can be decoded ahead, or since its last segment was decoded, whichever is later; `None` while it has not, or once it has ended.
    due: Option<Instant>,
    /// When its last segment was decoded.
    last: Option<Instant>,
    /// Where the next segment to decode starts; the audio before it has been decoded.
    decoded: usize,
    /// Where each segment ends: each segment decoded while the participant was speaking was found as it was decoded, and the rest once the first of them is due after the end.
    cuts: Vec<Cut>,
    /// What its sink was last told of its progress.
    told: Told,
    /// The transcript of each segment decoded so far, in order, without the empty ones and those left out as having no speech.
    texts: Vec<String>,
    /// How many segments have been decoded, how many of them while the participant was speaking, how many ended at their limit because there was no pause, and how many were left out of the transcript as having no speech in them.
    segments: usize,
    ahead: usize,
    hard: usize,
    skipped: usize,
}

impl OfflineEntry {
    /// Whether its next segment can be decoded while the participant is still speaking.
    fn decided(&self) -> bool {
        segment::decided(self.samples.len(), self.decoded)
    }
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
    /// Whether segments are decoded while the participant is still speaking: always, except in tests that compare with decoding only once utterances end.
    ahead: bool,
    pending: Pending,
}

impl<R: OfflineRecognizer> OfflineEngine<R> {
    fn new(
        commands: Receiver<Command>,
        recognizer: R,
        voice: Option<Box<dyn VoiceActivity>>,
        limit: usize,
        ahead: bool,
        pending: Pending,
    ) -> Self {
        Self {
            recognizer: Some(recognizer),
            segmenter: Segmenter { voice },
            commands,
            active: VecDeque::new(),
            disabled: false,
            limit,
            ahead,
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

    /// Apply the waiting commands, waiting for one if there is nothing to decode, then decode one segment; see [`OfflineEngine::step`]. Returns whether the engine keeps running.
    fn turn(&mut self) -> bool {
        if !self
            .active
            .iter()
            .any(|entry| entry.ready.is_some() || entry.due.is_some())
        {
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

    /// Decode the next segment of the utterance that has ended and waited longest for it: since the participant stopped speaking, or since its last segment was decoded. Utterances that have ended therefore take turns a segment at a time, and one that ends while another's long one is being decoded waits for at most one of its segments.
    ///
    /// While none has ended, decode ahead the next segment of the utterance still being spoken whose segment has been due longest, so that those being spoken take turns too. An utterance whose end is on its way, as its handle has said, goes first, so decoding ahead never starts once an utterance has ended; one that ends while a segment is decoded ahead is told that it waits.
    fn step(&mut self) {
        let ended = self
            .active
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| Some((index, entry.ready?)))
            .min_by_key(|&(_, ready)| ready)
            .map(|(index, _)| index);
        let Some(index) = ended.or_else(|| {
            self.active
                .iter()
                .enumerate()
                .filter_map(|(index, entry)| Some((index, entry.due?)))
                .min_by_key(|&(_, due)| due)
                .map(|(index, _)| index)
        }) else {
            return;
        };
        if ended.is_none() {
            let mut shared = pending(&self.pending);
            // An end on its way is in the commands, since a handle says it has ended as it sends it; the next turn applies it.
            if shared.open.iter().any(|(&key, &ended)| {
                ended
                    && !self
                        .active
                        .iter()
                        .any(|entry| entry.key == key && entry.ready.is_some())
            }) {
                return;
            }
            // An utterance that ends from now on waits for this segment, and is told so; one that ends while another that has ended is transcribed is told so for that.
            shared.decoding = Some(self.active[index].key);
        }
        let mut entry = self.active.remove(index).expect("index is in range");
        // The others that have ended now wait behind it, at least for this segment.
        for other in &mut self.active {
            if other.ready.is_some() && other.told != Told::Waiting {
                other.told = Told::Waiting;
                tell(&other.sink, TranscriptEvent::Progress(Progress::Waiting));
            }
        }
        let result = panic::catch_unwind(AssertUnwindSafe(|| self.advance(&mut entry)));
        // Including after a panic in the decode.
        pending(&self.pending).decoding = None;
        match result {
            Ok(true) => {
                let now = Instant::now();
                entry.last = Some(now);
                if entry.ready.is_some() {
                    entry.ready = Some(now);
                } else {
                    entry.due = entry.decided().then_some(now);
                }
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
        pending(&self.pending).open.remove(&entry.key);
        tell(&entry.sink, event);
    }

    /// Decode `entry`'s next segment, and report the transcript to its sink once every segment is decoded, or the failure; until then, report the transcript so far after a segment that added to it. Returns whether it has more to decode.
    fn advance(&mut self, entry: &mut OfflineEntry) -> bool {
        if entry.samples.is_empty() {
            // Nothing was said; the transcript is empty, as the streaming path reports it.
            self.conclude(entry, TranscriptEvent::Final(String::new()));
            return false;
        }
        let ended = entry.ready.is_some();
        if ended {
            // Every remaining segment is found at once, so that each can be told how many there are. The detector resets for each search, so finding them all first finds the same ones.
            let mut start = entry.cuts.last().map_or(0, |cut| cut.end());
            while start < entry.samples.len() {
                let cut = self.segmenter.cut(&entry.samples, start);
                start = cut.end();
                entry.cuts.push(cut);
            }
        } else {
            // While the participant speaks, only the next segment, whose end the audio so far decides: it ends where it would once they stop.
            entry
                .cuts
                .push(self.segmenter.cut(&entry.samples, entry.decoded));
            entry.ahead += 1;
        }
        let cut = entry.cuts[entry.segments];
        let parts = entry.cuts.len();
        // An utterance transcribed at once and whole needs no word of it: the server has said it is being transcribed. Nothing is said of one still being spoken.
        if ended && (parts > 1 || entry.told == Told::Waiting) {
            tell(
                &entry.sink,
                TranscriptEvent::Progress(Progress::Transcribing {
                    part: entry.segments + 1,
                    parts,
                }),
            );
        }
        if ended {
            entry.told = Told::Transcribing;
        }
        let end = cut.end();
        let segment = &entry.samples[entry.decoded..end];
        // A panic here is caught by `step`, which disables recognition and fails the utterance.
        let decoded = self.decode(segment);
        // Another utterance that ends now waits for nothing of one decoded ahead.
        pending(&self.pending).decoding = None;
        let text = match decoded {
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
            entry.decoded = end;
            return self.complete(entry, true);
        }
        entry.decoded = end;
        self.complete(entry, false)
    }

    /// Report `entry`'s transcript if every segment has been decoded, or, if its last segment `added` words, the transcript so far. Returns whether it has more to decode.
    fn complete(&mut self, entry: &mut OfflineEntry, added: bool) -> bool {
        if entry.decoded < entry.samples.len() {
            // Not to an utterance cancelled while its segment was decoded, whose words are no longer wanted. The handle forgets it before it sends the cancellation, so one cancelled from now on, as the sink is told, may still be told, and its server ignores it.
            if added && pending(&self.pending).open.contains_key(&entry.key) {
                tell(
                    &entry.sink,
                    TranscriptEvent::Partial(super::Transcript {
                        committed: entry.texts.join(" "),
                        tentative: String::new(),
                    }),
                );
            }
            return true;
        }
        if entry.segments > 1 {
            log::info!(
                "Utterance {}: {:.1} seconds transcribed in {} segments, {} of them while it was spoken, {} cut without a pause and {} left out as having no speech.",
                entry.key,
                entry.samples.len() as f64 / f64::from(RATE),
                entry.segments,
                entry.ahead,
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
                    pending(&self.pending).open.remove(&key);
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
                    due: None,
                    last: None,
                    decoded: 0,
                    cuts: Vec::new(),
                    told: Told::Nothing,
                    texts: Vec::new(),
                    segments: 0,
                    ahead: 0,
                    hard: 0,
                    skipped: 0,
                });
            }
            Command::Audio { key, samples, at } => {
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
                // Due from when its audio arrived, which may have been while another's segment was decoded.
                if self.ahead && entry.due.is_none() && entry.decided() {
                    entry.due = Some(at);
                }
            }
            // An utterance that failed while it was spoken, as too long, has nothing left to end.
            Command::End { key, at, waiting } => {
                if let Some(entry) = self.find(key) {
                    // A segment decoded ahead after the participant stopped, while the end was on its way, counts as its last turn.
                    entry.ready = Some(entry.last.map_or(at, |last| last.max(at)));
                    entry.due = None;
                    if waiting {
                        entry.told = Told::Waiting;
                    }
                }
            }
            Command::Cancel { key } => {
                self.active.retain(|entry| entry.key != key);
                pending(&self.pending).open.remove(&key);
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
        Self::start(recognizer, None, MAX_UTTERANCE, true)
    }

    /// Recognize with `recognizer`, splitting a clip longer than [`super::MAX_SEGMENT`] at the pauses `voice` hears, and by loudness where it hears none.
    pub fn with_voice_activity(
        recognizer: impl OfflineRecognizer + 'static,
        voice: impl VoiceActivity + 'static,
    ) -> Self {
        Self::start(recognizer, Some(Box::new(voice)), MAX_UTTERANCE, true)
    }

    /// Recognize with `recognizer` and `voice`, failing an utterance longer than `limit`, and decoding segments while the participant is still speaking if `ahead`.
    fn start(
        recognizer: impl OfflineRecognizer + 'static,
        voice: Option<Box<dyn VoiceActivity>>,
        limit: usize,
        ahead: bool,
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
                OfflineEngine::new(receiver, recognizer, voice, limit, ahead, shared).run();
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

    /// Begin an utterance whose audio arrives at `sample_rate`. Events are delivered to `sink` on the recognition thread, so it should not block, except [`Progress::Waiting`] as the utterance ends, which is delivered by [`OfflineUtterance::end`]; they are delivered one at a time, in order. A [`TranscriptEvent::Partial`] may come before the end, as segments are decoded while the participant speaks, and after it, between segments; progress comes only after the end.
    pub fn begin(
        &mut self,
        sample_rate: u32,
        sink: impl FnMut(TranscriptEvent) + Send + 'static,
    ) -> OfflineUtterance {
        self.next += 1;
        let key = self.next;
        let sink: Sink = Arc::new(Mutex::new(Box::new(sink)));
        pending(&self.pending).open.insert(key, false);
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
    /// Append audio at the utterance's sample rate. It is buffered, and each segment is decoded once the audio has decided where it ends, the rest once the utterance ends.
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
            at: Instant::now(),
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

    /// The participant finished speaking: decode the rest of the buffered clip and deliver the final transcript. If another utterance that has ended is still being transcribed, or another's segment is being decoded while it is spoken, the sink is first told, before this returns, that this one waits.
    pub fn end(mut self) {
        self.open = false;
        // The sink is held while the end is sent and the wait told, so that nothing the engine tells of this utterance comes first, and the engine has the end once the wait is told.
        let mut sink = self.sink.lock().unwrap_or_else(PoisonError::into_inner);
        let waiting = {
            // Taken together, and the end sent meanwhile, so that the utterances that end first are first in line, and the engine, which decodes ahead only while no utterance has ended, either sees this one ended, with its end among its commands, or has started decoding ahead, which this one waits for. One that has already failed waits for nothing.
            let mut shared = pending(&self.pending);
            let ended = shared.open.get_mut(&self.key).is_some_and(|ended| {
                *ended = true;
                true
            });
            let others = shared
                .open
                .iter()
                .any(|(&key, &ended)| ended && key != self.key)
                || shared.decoding.is_some_and(|key| key != self.key);
            let waiting = ended && others;
            let _ = self.commands.send(Command::End {
                key: self.key,
                at: Instant::now(),
                waiting,
            });
            waiting
        };
        if waiting {
            sink(TranscriptEvent::Progress(Progress::Waiting));
        }
    }

    /// Discard the utterance without a transcript, and stop decoding it. No further events are delivered, except one already being delivered.
    pub fn cancel(mut self) {
        self.open = false;
        self.forget();
    }

    /// Forget the utterance, so that the engine reports nothing more of it, not even the words of a segment it is decoding, and tell the engine to drop it.
    fn forget(&self) {
        pending(&self.pending).open.remove(&self.key);
        let _ = self.commands.send(Command::Cancel { key: self.key });
    }
}

impl Drop for OfflineUtterance {
    fn drop(&mut self) {
        if self.open {
            self.forget();
        }
    }
}

#[cfg(test)]
mod tests;
