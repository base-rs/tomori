// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The agent speech stream of each space. See `design/speech.md` and `design/live-audio.md`.
//!
//! A space's driver feeds every backend event to its [`Voice`]. Each assistant message's text deltas pass through a [`tomori_speech::Speakable`], and the closed document-level blocks are queued, in order, as segments on the space's one [`tomori_speech::Speech`]. A [`SpeechMode`] decides how a closed block is split into segments: one per block, its sentences joined, so the voice keeps the paragraph's cadence (the default); or one per sentence, so a long block starts speaking while it is still streaming. Nothing participants do stops, pauses or skips queued speech.
//!
//! Synthesis runs ahead of playback on the speech crate's thread. A pacer thread per space announces each segment with [`tomori_protocol::ServerMessage::Speech`] and then sends its audio as 20 ms [`tomori_protocol::AudioFrame`]s from [`tomori_protocol::AGENT_SOURCE`] at the rate it plays, slightly ahead, to whoever in the space hears live speech at that moment. A breath of silence follows each segment, so that back-to-back blocks do not run together. Only that thread touches the hub, and only briefly to queue each frame.
//!
//! The hub keeps what is sent, numbered by reply, one reply per agent turn, so that a participant who missed it hears it from its start; see [`crate::retention`]. The same pacer thread paces those catch-ups and replays, woken when one starts.
//!
//! With the space's speech setting `brief`, each assistant message speaks its first blocks, up to about [`BRIEF_CHARACTERS`], and leaves out the rest; the first message of a reply cut short says "More on the panel." instead, once per reply.

/// About 20 seconds of speech: Kokoro's `af_heart` voice speaks about 15 characters of English a second. A brief message speaks whole blocks while they fit, then stops at the next block boundary; a block that starts within the first half of this, such as the paragraph after a short heading or lead-in, is cut at a sentence boundary instead, and the first block always speaks at least one sentence.
#[cfg_attr(not(feature = "speech"), allow(dead_code))]
pub const BRIEF_CHARACTERS: usize = 300;

/// Spoken in place of the rest of a reply cut short by brief speech.
#[cfg_attr(not(feature = "speech"), allow(dead_code))]
pub const MORE_ON_THE_PANEL: &str = "More on the panel.";

/// Which sentences of `blocks` a brief message speaks, given the characters `spent` on its earlier blocks, and whether it is cut there. Each block's sentences are returned, empty for those not spoken.
#[cfg_attr(not(feature = "speech"), allow(dead_code))]
fn brief(blocks: &[Vec<String>], spent: &mut usize, cut: &mut bool) -> Vec<Vec<String>> {
    let characters = |text: &String| text.chars().count();
    let mut spoken = Vec::new();
    for sentences in blocks {
        let length: usize = sentences.iter().map(characters).sum();
        if *cut || sentences.is_empty() {
            spoken.push(Vec::new());
        } else if *spent + length <= BRIEF_CHARACTERS {
            *spent += length;
            spoken.push(sentences.clone());
        } else if *spent < BRIEF_CHARACTERS / 2 {
            // A block too long to fit that starts early, as after a heading or lead-in, is cut at a sentence; the first block speaks at least one.
            let mut taken = Vec::new();
            for sentence in sentences {
                let first = taken.is_empty() && *spent == 0;
                if !first && *spent + characters(sentence) > BRIEF_CHARACTERS {
                    break;
                }
                *spent += characters(sentence);
                taken.push(sentence.clone());
            }
            *cut = taken.len() < sentences.len();
            spoken.push(taken);
        } else {
            *cut = true;
            spoken.push(Vec::new());
        }
    }
    spoken
}

/// How a space splits a closed document-level block into speech segments.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SpeechMode {
    /// One segment per closed block, its speakable sentences joined: the voice keeps the cadence of the paragraph, list or other block it came from.
    #[default]
    Block,
    /// One segment per sentence, so that the first sentence of a long block starts speaking while the rest of the block is still streaming.
    Sentence,
}

#[cfg(feature = "speech")]
pub(crate) use enabled::{Voice, Voices};

#[cfg(not(feature = "speech"))]
pub(crate) use disabled::{Voice, Voices};

#[cfg(not(feature = "speech"))]
mod disabled {
    use std::sync::Arc;

    use tomori_backend::Event;

    use crate::hub::Hub;

    /// Without the `speech` feature, spaces have no agent speech.
    #[derive(Clone, Default)]
    pub struct Voices(());

    impl Voices {
        pub fn start(&self, _hub: &Arc<Hub>, _space: &str) -> Option<Voice> {
            None
        }
    }

    pub enum Voice {}

    impl Voice {
        pub fn observe(&mut self, _event: &Event) {
            match *self {}
        }
    }
}

#[cfg(feature = "speech")]
mod enabled {
    use std::collections::{HashMap, VecDeque};
    use std::panic::{self, AssertUnwindSafe};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
    use std::sync::{Arc, Mutex, PoisonError, Weak};
    use std::thread;
    use std::time::{Duration, Instant};

    use tomori_backend::{Event, Role};
    use tomori_protocol::{AGENT_SOURCE, AudioFrame, Gate, ServerMessage, SpeechSetting};
    use tomori_speech::{
        AudioChunk, Block, Error, Speakable, SpeakableOptions, Speech, SpeechEvent, Synthesizer,
    };

    use super::{MORE_ON_THE_PANEL, SpeechMode};
    use crate::hub::Hub;
    use crate::retention::{LEAD, SEGMENT_GAP};

    /// Frames per second of agent speech: 20 ms frames.
    const FRAMES_PER_SECOND: u32 = 50;

    /// How often a pacer that keeps falling behind says so, at most, as the clients' logging does (see
    /// `design/client-logging.md`).
    const BEHIND_REPORT: Duration = Duration::from_secs(30);

    /// The longest piece of text in one speech segment. In the `block` mode, synthesis is in whole
    /// document-level blocks, as the incremental parser hands them over, so a block keeps the
    /// cadence of the paragraph it came from; the limit only splits the pathological block, a single
    /// line of several thousand characters, at a word. In the `sentence` mode, each sentence is its
    /// own segment and the limit is not the normal case.
    const MAXIMUM_SEGMENT: usize = 2100;

    /// The speakable filter of an assistant message, splitting at most [`MAXIMUM_SEGMENT`] bytes at a time.
    fn speakable() -> Speakable {
        Speakable::with_options(SpeakableOptions {
            code_notice: None,
            maximum_segment: MAXIMUM_SEGMENT,
        })
    }

    /// The synthesizer shared by every space. Spaces take turns segment by segment; each space's own segments stay in order.
    ///
    /// A panic in the synthesizer disables it for every space, since it may be left inconsistent; the space whose segment panicked reports the panic, and the others fail with [`Error::Stopped`].
    #[derive(Clone)]
    struct Shared(Arc<Engine>);

    struct Engine {
        synthesizer: Mutex<Box<dyn Synthesizer>>,
        disabled: AtomicBool,
    }

    impl Synthesizer for Shared {
        fn synthesize(
            &mut self,
            text: &str,
            output: &mut dyn FnMut(AudioChunk),
        ) -> Result<(), Error> {
            let engine = &*self.0;
            let mut synthesizer = engine
                .synthesizer
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if engine.disabled.load(Ordering::Acquire) {
                return Err(Error::Stopped(
                    "speech synthesis stopped after an internal error".into(),
                ));
            }
            panic::catch_unwind(AssertUnwindSafe(|| synthesizer.synthesize(text, output)))
                .unwrap_or_else(|panic| {
                    engine.disabled.store(true, Ordering::Release);
                    panic::resume_unwind(panic)
                })
        }
    }

    /// Starts the agent speech stream of each space, if the server has a synthesizer.
    #[derive(Clone, Default)]
    pub struct Voices {
        synthesizer: Option<Shared>,
        mode: SpeechMode,
    }

    impl Voices {
        pub fn new(synthesizer: impl Synthesizer + 'static) -> Self {
            Self {
                synthesizer: Some(Shared(Arc::new(Engine {
                    synthesizer: Mutex::new(Box::new(synthesizer)),
                    disabled: AtomicBool::new(false),
                }))),
                mode: SpeechMode::default(),
            }
        }

        /// How closed blocks are split into segments: one per block (the default), or one per sentence.
        pub fn with_mode(mut self, mode: SpeechMode) -> Self {
            self.mode = mode;
            self
        }

        /// Start the speech stream of `space`, or `None` when the server has no synthesizer.
        pub fn start(&self, hub: &Arc<Hub>, space: &str) -> Option<Voice> {
            let synthesizer = self.synthesizer.clone()?;
            let (sender, events) = mpsc::channel();
            let waker = sender.clone();
            hub.set_speech_waker(
                space,
                Box::new(move || {
                    let _ = waker.send(Input::Wake);
                }),
            );
            let labels = Arc::new(Mutex::new(HashMap::new()));
            let closing = Arc::new(AtomicBool::new(false));
            let pacer = Pacer::new(
                Arc::downgrade(hub),
                space.to_owned(),
                labels.clone(),
                closing.clone(),
                events,
            );
            // As the synthesis thread started below, a thread the system cannot start is not survived.
            thread::Builder::new()
                .name("speech-pacer".into())
                .spawn(move || pacer.run())
                .expect("spawn the speech pacer");
            // Tells the pacer when synthesis has stopped, as the hub's waker keeps the channel open.
            let ended = Ended(sender);
            let speech = Speech::spawn(synthesizer, move |event| {
                // The pacer only stops once the hub is gone, after which speech has nowhere to go.
                let _ = ended.0.send(Input::Speech(event));
            });
            Some(Voice {
                speech,
                labels,
                closing,
                mode: self.mode,
                speaking: HashMap::new(),
                hub: Arc::downgrade(hub),
                space: space.to_owned(),
                reply: 0,
                idle: true,
                more: None,
            })
        }
    }

    /// What the pacer is told: synthesis events, that a replay or catch-up started, or that synthesis has stopped.
    enum Input {
        Speech(SpeechEvent),
        Wake,
        Ended,
    }

    /// Sends [`Input::Ended`] when synthesis drops its event callback, which owns this.
    struct Ended(Sender<Input>);

    impl Drop for Ended {
        fn drop(&mut self) {
            let _ = self.0.send(Input::Ended);
        }
    }

    /// An assistant message being spoken.
    struct Spoken {
        speakable: Speakable,
        /// For a brief reply, the characters spoken so far and whether the rest is cut.
        brief: Option<(usize, bool)>,
    }

    /// The agent speech stream of one space, fed with the space's backend events.
    ///
    /// Only assistant messages that start while the space is running are spoken: their text deltas from `MessageStarted` to `MessageCompleted`. History from the session snapshot, user messages, reasoning and tool activity are never spoken.
    pub struct Voice {
        speech: Speech,
        /// The message and reply of each queued segment, for its announcement.
        labels: Arc<Mutex<HashMap<u64, (String, u64)>>>,
        /// Set when the space stops, so that the pacer can tell an orderly end from a failure.
        closing: Arc<AtomicBool>,
        /// How closed blocks are split into segments.
        mode: SpeechMode,
        /// Assistant messages being spoken.
        speaking: HashMap<String, Spoken>,
        /// The hub and space, for the space's speech setting.
        hub: Weak<Hub>,
        space: String,
        /// The current reply's number, counting agent turns: one turn's speech is one reply, from its first assistant message after the agent was idle.
        reply: u64,
        /// Whether the agent has been idle since the current reply started, so the next assistant message starts a reply. A busy state the backend repeats, as when it reads the session again, does not.
        idle: bool,
        /// The reply that has said "More on the panel.", so that it is said once per reply.
        more: Option<u64>,
    }

    impl Voice {
        pub fn observe(&mut self, event: &Event) {
            match event {
                Event::Idle => self.idle = true,
                Event::MessageStarted {
                    message,
                    role: Role::Assistant,
                } => {
                    // A message already being spoken, as when the backend reads the session again, continues its reply.
                    if !self.speaking.contains_key(message) {
                        if std::mem::take(&mut self.idle) {
                            self.reply += 1;
                        }
                        let brief = self.hub.upgrade().is_some_and(|hub| {
                            hub.speech_setting(&self.space) == SpeechSetting::Brief
                        });
                        self.speaking.insert(
                            message.clone(),
                            Spoken {
                                speakable: speakable(),
                                brief: brief.then_some((0, false)),
                            },
                        );
                    }
                }
                Event::TextDelta { message, text } => {
                    if let Some(spoken) = self.speaking.get_mut(message) {
                        let blocks = spoken.speakable.push(text);
                        self.speak(message, blocks);
                    }
                }
                Event::MessageCompleted { message } => {
                    if let Some(spoken) = self.speaking.get_mut(message) {
                        let blocks = spoken.speakable.finish();
                        self.speak(message, blocks);
                        self.speaking.remove(message);
                    }
                }
                // An incomplete message's unclosed block is not stable, so it is not spoken. Queued segments still finish.
                // After a resynchronization the deltas of a message being spoken were lost, so the rest of it is not
                // spoken either; messages that start afterwards are.
                Event::Closed { .. } | Event::Resynchronized { .. } => self.speaking.clear(),
                _ => {}
            }
        }

        fn speak(&mut self, message: &str, blocks: Vec<Block>) {
            let mut sentences: Vec<Vec<String>> =
                blocks.into_iter().map(|block| block.sentences).collect();
            let mut more = false;
            if let Some((spent, cut)) = self
                .speaking
                .get_mut(message)
                .and_then(|spoken| spoken.brief.as_mut())
            {
                let was_cut = *cut;
                sentences = super::brief(&sentences, spent, cut);
                more = *cut && !was_cut;
            }
            let mut segments = self.segments(&sentences);
            if more && self.more != Some(self.reply) {
                self.more = Some(self.reply);
                segments.push(MORE_ON_THE_PANEL.to_owned());
            }
            // Holding the labels across `speak` means the pacer finds each segment's label once synthesis starts it.
            let mut labels = self.labels.lock().unwrap_or_else(PoisonError::into_inner);
            // A segment with nothing to say is not spoken.
            for segment in segments
                .iter()
                .filter_map(|segment| self.speech.speak(segment))
            {
                labels.insert(segment, (message.to_owned(), self.reply));
            }
        }

        /// The text of each speech segment of the blocks' `sentences`, in the voice's mode.
        ///
        /// In `block` mode a block is one segment, its speakable sentences joined, so the voice keeps the paragraph's cadence. In `sentence` mode each sentence is its own segment, so the first sentence of a long block starts while the rest of the block is still streaming. A block with nothing to say yields no segment.
        fn segments(&self, sentences: &[Vec<String>]) -> Vec<String> {
            let mut segments = Vec::new();
            for block in sentences {
                match self.mode {
                    SpeechMode::Block => {
                        if !block.is_empty() {
                            segments.push(block.join(" "));
                        }
                    }
                    SpeechMode::Sentence => segments.extend(block.iter().cloned()),
                }
            }
            segments
        }
    }

    impl Drop for Voice {
        /// Queued segments are still synthesized and played; see [`Speech`].
        fn drop(&mut self) {
            self.closing.store(true, Ordering::SeqCst);
        }
    }

    enum Item {
        Announce(ServerMessage),
        Frame {
            sample_rate: u32,
            samples: Vec<i16>,
        },
        /// A breath of [`SEGMENT_GAP`] of silence between two spoken segments.
        Gap,
        Failed(String),
    }

    /// Turns synthesized audio into a real-time frame stream for one space, and paces its participants' catch-ups and replays.
    struct Pacer {
        hub: Weak<Hub>,
        space: String,
        labels: Arc<Mutex<HashMap<u64, (String, u64)>>>,
        closing: Arc<AtomicBool>,
        events: Receiver<Input>,
        /// Whether synthesis is still running.
        connected: bool,
        /// Whether the hub can still wake this pacer, which it can while the space exists.
        open: bool,
        /// Items to send, each with its reply.
        pending: VecDeque<(u64, Item)>,
        /// The segment that started but has produced no audio yet: its number, message and text.
        announcing: Option<(u64, String, String)>,
        /// Whether the current segment has produced audio, so that a breath follows it.
        spoke: bool,
        /// The reply of the current segment.
        reply: u64,
        /// When the hub next has catch-up or replay audio due, if anyone is catching up or replaying.
        streams: Option<Instant>,
        /// A catch-up or replay started, so the hub has audio due now.
        woken: bool,
        /// Samples short of a whole frame, at `carry_rate`.
        carry: Vec<i16>,
        carry_rate: u32,
        /// When the next frame should start playing. Behind the current time after the stream has been idle.
        clock: Option<Instant>,
        epoch: Instant,
        sequence: u32,
        /// Whether synthesis has been reported as stopped.
        stopped: bool,
        /// Whether the last item sent was a frame, so that the next frame of the segment is due as it ends.
        framing: bool,
        /// Whether, since then, the pacer had nothing more of the segment to send, waiting for synthesis.
        starved: bool,
        /// Frames of a segment sent after they were due to play, and how far behind, since that was last said.
        behind: Behind,
    }

    /// Frames a pacer sent after they were due to play, so that listeners heard a gap: when it last said so, and how many
    /// more since, of them how many waited for synthesis, and the furthest behind of them.
    #[derive(Debug, Default)]
    struct Behind {
        said: Option<Instant>,
        count: u32,
        starved: u32,
        worst: Duration,
    }

    impl Behind {
        /// A frame was sent `late` after it was due to play, at `now`, after waiting for synthesis if `starved`, as when
        /// the server is too busy to synthesize in time, or else held up, as when its thread or the hub's lock was: what
        /// to log, at most once every [`BEHIND_REPORT`], counting those in between.
        fn late(&mut self, late: Duration, starved: bool, now: Instant) -> Option<String> {
            self.count += 1;
            self.starved += u32::from(starved);
            self.worst = self.worst.max(late);
            if self
                .said
                .is_some_and(|said| now.saturating_duration_since(said) < BEHIND_REPORT)
            {
                return None;
            }
            let said = format!(
                "fell {} ms behind its playback{}, so listeners heard a gap",
                self.worst.as_millis(),
                match (self.count, self.starved) {
                    (1, 1) => " waiting for synthesis".to_owned(),
                    (1, _) => ", held up".to_owned(),
                    (count, starved) => {
                        format!(" {count} times, {starved} of them waiting for synthesis")
                    }
                }
            );
            *self = Self {
                said: Some(now),
                ..Self::default()
            };
            Some(said)
        }
    }

    /// Reported once per space when synthesis has stopped: a panic disabled it, or the server is shutting down.
    const SYNTHESIS_STOPPED: &str =
        "Speech synthesis is unavailable until the server restarts, so replies are not spoken.";

    impl Pacer {
        fn new(
            hub: Weak<Hub>,
            space: String,
            labels: Arc<Mutex<HashMap<u64, (String, u64)>>>,
            closing: Arc<AtomicBool>,
            events: Receiver<Input>,
        ) -> Self {
            Self {
                hub,
                space,
                labels,
                closing,
                events,
                connected: true,
                open: true,
                pending: VecDeque::new(),
                announcing: None,
                spoke: false,
                reply: 0,
                streams: None,
                woken: false,
                carry: Vec::new(),
                carry_rate: 0,
                clock: None,
                epoch: Instant::now(),
                sequence: 0,
                stopped: false,
                framing: false,
                starved: false,
                behind: Behind::default(),
            }
        }

        fn run(mut self) {
            loop {
                let now = Instant::now();
                if self.woken || self.streams.is_some_and(|due| due <= now) {
                    self.woken = false;
                    let Some(hub) = self.hub.upgrade() else {
                        return;
                    };
                    match hub.pace_speech(&self.space, now) {
                        Some(next) => self.streams = next,
                        // The space is gone, so nobody is left to hear it.
                        None => return,
                    }
                }
                let live = match self.pending.front() {
                    // Part way through a segment, the next of it is still being synthesized.
                    None => {
                        self.starved |= self.framing;
                        None
                    }
                    Some((_, Item::Frame { .. })) => {
                        let clock = self.clock.filter(|clock| *clock >= now).unwrap_or(now);
                        let due = clock.checked_sub(LEAD).unwrap_or(clock);
                        Some(due.saturating_duration_since(now))
                    }
                    Some(_) => Some(Duration::ZERO),
                };
                if live == Some(Duration::ZERO) {
                    let (reply, item) = self.pending.pop_front().expect("checked above");
                    if !self.send(reply, item, now) {
                        return;
                    }
                    continue;
                }
                let streams = self.streams.map(|due| due.saturating_duration_since(now));
                let wait = match (live, streams) {
                    (Some(live), Some(streams)) => Some(live.min(streams)),
                    (live, streams) => live.or(streams),
                };
                if !self.open {
                    // Only what is already queued is left to send, and nothing waits once nothing is queued.
                    let Some(wait) = wait else {
                        return;
                    };
                    thread::sleep(wait);
                    continue;
                }
                let received = match wait {
                    None => self
                        .events
                        .recv()
                        .map_err(|_| RecvTimeoutError::Disconnected),
                    Some(wait) => self.events.recv_timeout(wait),
                };
                match received {
                    Ok(Input::Speech(event)) => self.handle(event),
                    Ok(Input::Wake) => self.woken = true,
                    Ok(Input::Ended) => self.disconnect(),
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => {
                        self.open = false;
                        self.disconnect();
                    }
                }
            }
        }

        /// Send one item of reply number `reply` to the space. Returns false once the server has stopped.
        fn send(&mut self, reply: u64, item: Item, now: Instant) -> bool {
            let Some(hub) = self.hub.upgrade() else {
                return false;
            };
            // A frame that follows another of its segment was due as that one ended; sent after that, it plays late, so
            // listeners hear a gap, as when this thread, synthesis or the hub's lock held it back beyond [`LEAD`].
            if let (Item::Frame { .. }, true, Some(clock)) = (&item, self.framing, self.clock)
                && let Some(late) = now
                    .checked_duration_since(clock)
                    .filter(|late| !late.is_zero())
                && let Some(said) = self.behind.late(late, self.starved, now)
            {
                log::info!("Agent speech in {} {said}.", self.space);
            }
            self.framing = matches!(item, Item::Frame { .. });
            self.starved = false;
            match item {
                Item::Announce(message) => hub.agent_speech(&self.space, reply, message),
                Item::Failed(error) => hub.speech_failed(&self.space, error),
                Item::Gap => {
                    // A breath between segments: advance the clock so the next frame, and the
                    // silence the client plays while it waits for it, is later by [`SEGMENT_GAP`].
                    let clock = self.clock.filter(|clock| *clock >= now).unwrap_or(now);
                    self.clock = Some(clock + SEGMENT_GAP);
                    hub.agent_gap(&self.space, reply);
                }
                Item::Frame {
                    sample_rate,
                    samples,
                } => {
                    let clock = self.clock.filter(|clock| *clock >= now).unwrap_or(now);
                    let duration =
                        Duration::from_secs_f64(samples.len() as f64 / f64::from(sample_rate));
                    let frame = AudioFrame {
                        gate: Gate::Live,
                        source: AGENT_SOURCE,
                        sequence: self.sequence,
                        timestamp: clock.duration_since(self.epoch).as_micros() as u64,
                        sample_rate,
                        samples,
                    };
                    self.sequence = self.sequence.wrapping_add(1);
                    self.clock = Some(clock + duration);
                    hub.agent_audio(&self.space, reply, &frame);
                }
            }
            true
        }

        fn handle(&mut self, event: SpeechEvent) {
            match event {
                SpeechEvent::Started { segment, text } => {
                    self.flush();
                    let (message, reply) = self
                        .labels
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .remove(&segment)
                        .unwrap_or((String::new(), self.reply));
                    self.reply = reply;
                    self.announcing = Some((segment, message, text));
                }
                SpeechEvent::Audio { chunk, .. } => self.audio(chunk),
                SpeechEvent::Finished { .. } => {
                    self.flush();
                    self.announcing = None;
                    if std::mem::take(&mut self.spoke) {
                        // A breath between this segment and the next, so back-to-back blocks do not run together.
                        self.pending.push_back((self.reply, Item::Gap));
                    }
                }
                SpeechEvent::Failed { error, .. } => {
                    self.flush();
                    self.announcing = None;
                    self.spoke = false;
                    if matches!(error, tomori_speech::Error::Stopped(_)) {
                        // Synthesis stays stopped until the server restarts, so say so once rather than for every sentence.
                        if !self.stopped {
                            self.stopped = true;
                            self.pending.push_back((
                                self.reply,
                                Item::Failed(SYNTHESIS_STOPPED.to_owned()),
                            ));
                        }
                    } else {
                        self.pending.push_back((
                            self.reply,
                            Item::Failed(format!(
                                "Part of the reply could not be spoken: {error}."
                            )),
                        ));
                    }
                }
            }
        }

        fn audio(&mut self, chunk: AudioChunk) {
            if chunk.sample_rate == 0 {
                return;
            }
            if chunk.sample_rate != self.carry_rate {
                self.flush();
                self.carry_rate = chunk.sample_rate;
            }
            if let Some((segment, message, text)) = self.announcing.take() {
                self.spoke = true;
                self.pending.push_back((
                    self.reply,
                    Item::Announce(ServerMessage::Speech {
                        segment: u32::try_from(segment).unwrap_or(u32::MAX),
                        message,
                        text,
                        sample_rate: chunk.sample_rate,
                        replay: false,
                    }),
                ));
            }
            self.carry.extend_from_slice(&chunk.samples);
            let size = (chunk.sample_rate / FRAMES_PER_SECOND).max(1) as usize;
            while self.carry.len() >= size {
                let samples = self.carry.drain(..size).collect();
                self.pending.push_back((
                    self.reply,
                    Item::Frame {
                        sample_rate: chunk.sample_rate,
                        samples,
                    },
                ));
            }
        }

        /// Send the rest of a segment as a short frame.
        fn flush(&mut self) {
            if !self.carry.is_empty() {
                self.pending.push_back((
                    self.reply,
                    Item::Frame {
                        sample_rate: self.carry_rate,
                        samples: std::mem::take(&mut self.carry),
                    },
                ));
            }
        }

        /// Synthesis has stopped: send what it produced, and report it unless the space is closing.
        fn disconnect(&mut self) {
            if !self.connected {
                return;
            }
            self.connected = false;
            self.flush();
            if !self.closing.load(Ordering::SeqCst) {
                self.pending.push_back((
                    self.reply,
                    Item::Failed(
                        "Agent speech stopped unexpectedly; replies in this space will not be spoken."
                            .to_owned(),
                    ),
                ));
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::backends::Backends;
        use crate::transcriber::NullTranscriber;
        use tomori_speech::{AudioChunk, Synthesizer};

        /// A synthesizer that says nothing.
        struct Mute;

        impl Synthesizer for Mute {
            fn synthesize(
                &mut self,
                _: &str,
                _: &mut dyn FnMut(AudioChunk),
            ) -> Result<(), tomori_speech::Error> {
                Ok(())
            }
        }

        /// A hub with a space nobody is in, and the space.
        fn space() -> (Arc<Hub>, String) {
            let (jobs, _) = async_channel::unbounded();
            let hub = Hub::new(
                Backends::new(),
                Arc::new(NullTranscriber::new()),
                jobs,
                Vec::new(),
                crate::workspace::SpaceSlots::new(1),
            );
            let (commands, _) = async_channel::unbounded();
            let request = crate::hub::CreateRequest {
                title: "Space".into(),
                backend: "fake".into(),
                directory: "/tmp".into(),
                session: None,
                creator: "nobody".into(),
                creator_space: None,
                environment: "sky".into(),
                instructions: String::new(),
            };
            let id = hub
                .register_space(&request, "session".into(), Default::default(), commands)
                .unwrap();
            (hub, id)
        }

        /// A pacer for `space` of `hub`, with what tells it of synthesis.
        fn pacer(hub: &Arc<Hub>, space: &str) -> (Pacer, Sender<Input>) {
            let (sender, events) = mpsc::channel();
            let pacer = Pacer::new(
                Arc::downgrade(hub),
                space.into(),
                Arc::default(),
                Arc::new(AtomicBool::new(false)),
                events,
            );
            (pacer, sender)
        }

        fn chunk(sample_rate: u32, samples: usize) -> AudioChunk {
            AudioChunk {
                sample_rate,
                samples: vec![0; samples],
            }
        }

        #[test]
        fn a_breath_follows_each_spoken_segment_and_not_a_silent_one() {
            let (hub, space) = space();
            let (mut pacer, _sender) = pacer(&hub, &space);
            let breaths = |pacer: &Pacer| {
                pacer
                    .pending
                    .iter()
                    .filter(|(_, item)| matches!(item, Item::Gap))
                    .count()
            };
            pacer.handle(SpeechEvent::Started {
                segment: 1,
                text: "Hello.".into(),
            });
            pacer.audio(chunk(16_000, 400));
            pacer.handle(SpeechEvent::Finished { segment: 1 });
            assert_eq!(breaths(&pacer), 1);
            pacer.handle(SpeechEvent::Started {
                segment: 2,
                text: "...".into(),
            });
            pacer.handle(SpeechEvent::Finished { segment: 2 });
            assert_eq!(breaths(&pacer), 1);
        }

        #[test]
        fn a_segment_becomes_an_announcement_frames_and_a_breath_and_failures_are_said() {
            let (hub, space) = space();
            let (mut pacer, _sender) = pacer(&hub, &space);
            pacer.handle(SpeechEvent::Started {
                segment: 1,
                text: "Hello.".into(),
            });
            // Audio without a rate is nothing.
            pacer.audio(chunk(0, 10));
            pacer.audio(chunk(16_000, 400));
            // Another rate starts its frames afresh.
            pacer.audio(chunk(24_000, 10));
            pacer.handle(SpeechEvent::Finished { segment: 1 });
            // A segment that never started takes no breath.
            pacer.handle(SpeechEvent::Finished { segment: 2 });
            for error in [
                tomori_speech::Error::Engine("bad text".into()),
                tomori_speech::Error::Stopped("gone".into()),
                tomori_speech::Error::Stopped("gone".into()),
            ] {
                pacer.handle(SpeechEvent::Failed { segment: 3, error });
            }
            let kinds: Vec<&str> = pacer
                .pending
                .iter()
                .map(|(_, item)| match item {
                    Item::Announce(_) => "announce",
                    Item::Frame { .. } => "frame",
                    Item::Gap => "gap",
                    Item::Failed(_) => "failed",
                })
                .collect();
            assert_eq!(
                kinds,
                [
                    "announce", "frame", "frame", "frame", "gap", "failed", "failed"
                ]
            );
            let now = Instant::now();
            while let Some((reply, item)) = pacer.pending.pop_front() {
                assert!(pacer.send(reply, item, now));
            }
        }

        #[test]
        fn synthesis_that_stops_is_said_unless_the_space_is_closing() {
            let (hub, space) = space();
            let (mut pacer, _sender) = self::pacer(&hub, &space);
            pacer.audio(chunk(16_000, 10));
            pacer.disconnect();
            pacer.disconnect();
            let Some((_, Item::Failed(_))) = pacer.pending.back() else {
                unreachable!("stopping is said after the frame");
            };
            let (mut closing, _sender) = self::pacer(&hub, &space);
            closing.closing.store(true, Ordering::SeqCst);
            closing.disconnect();
            assert!(closing.pending.is_empty());
        }

        #[test]
        fn the_pacer_stops_once_its_space_or_its_server_is_gone() {
            let (hub, space) = space();
            let (mut gone, sender) = pacer(&hub, "gone");
            gone.woken = true;
            gone.run();
            drop(sender);
            let (mut stopped, sender) = pacer(&hub, &space);
            stopped.pending.push_back((0, Item::Gap));
            let (mut woken, woken_sender) = pacer(&hub, &space);
            woken.woken = true;
            drop(hub);
            stopped.run();
            woken.run();
            drop((sender, woken_sender));
        }

        #[test]
        fn a_frame_sent_after_it_was_due_to_play_is_said_at_most_every_thirty_seconds() {
            let (hub, space) = space();
            let (mut pacer, _sender) = pacer(&hub, &space);
            // Six frames of 20 ms.
            pacer.audio(chunk(16_000, 1_920));
            let start = Instant::now();
            let ms = |ms: u64| start + Duration::from_millis(ms);
            let send = |pacer: &mut Pacer, at: Instant| {
                let (reply, item) = pacer.pending.pop_front().unwrap();
                assert!(pacer.send(reply, item, at));
            };
            // The first frame of a segment starts the clock, however late it is.
            send(&mut pacer, start);
            assert_eq!(pacer.behind.count, 0);
            assert!(pacer.behind.said.is_none());
            // The next was due as the first ended, at 20 ms: sent at 70 ms, it is said at once.
            send(&mut pacer, ms(70));
            assert_eq!(pacer.behind.said, Some(ms(70)));
            assert_eq!(pacer.behind.count, 0);
            // Sent ahead of when it plays, at 90 ms, nothing is behind.
            send(&mut pacer, ms(80));
            assert_eq!(pacer.behind.count, 0);
            // Late again within thirty seconds, 190 ms after it was due and then 80 ms, each is counted, and the furthest
            // behind kept, rather than said.
            send(&mut pacer, ms(300));
            send(&mut pacer, ms(400));
            assert_eq!(
                (pacer.behind.count, pacer.behind.worst),
                (2, Duration::from_millis(190))
            );
            // Thirty seconds on, the next is said with those.
            send(&mut pacer, ms(30_100));
            assert_eq!(pacer.behind.said, Some(ms(30_100)));
            assert_eq!(pacer.behind.count, 0);
            // A breath and a new segment start the clock again.
            pacer.pending.push_back((0, Item::Gap));
            send(&mut pacer, ms(40_000));
            assert!(!pacer.framing);
        }

        #[test]
        fn falling_behind_says_how_far_and_how_often() {
            let now = Instant::now();
            let mut behind = Behind::default();
            assert_eq!(
                behind
                    .late(Duration::from_millis(120), false, now)
                    .as_deref(),
                Some("fell 120 ms behind its playback, held up, so listeners heard a gap")
            );
            assert_eq!(behind.late(Duration::from_millis(40), true, now), None);
            assert_eq!(
                behind
                    .late(Duration::from_millis(30), false, now + BEHIND_REPORT)
                    .as_deref(),
                Some(
                    "fell 40 ms behind its playback 2 times, 1 of them waiting for synthesis, so listeners heard a gap"
                )
            );
            assert_eq!(
                behind
                    .late(Duration::from_millis(60), true, now + BEHIND_REPORT * 2)
                    .as_deref(),
                Some(
                    "fell 60 ms behind its playback waiting for synthesis, so listeners heard a gap"
                )
            );
        }

        #[test]
        fn the_pacer_sends_what_is_queued_once_nothing_can_wake_it() {
            let (hub, space) = space();
            let (mut pacer, sender) = pacer(&hub, &space);
            drop(sender);
            // Twice the lead, so that it waits for the later frames to be due.
            pacer.audio(chunk(16_000, (2 * LEAD.as_millis() as usize) * 16));
            // Returns once the frames are sent.
            pacer.run();
        }

        #[test]
        fn a_message_is_spoken_once_and_what_is_not_spoken_is_ignored() {
            let (hub, space) = space();
            let mut voice = Voices::new(Mute).start(&hub, &space).unwrap();
            let started = |message: &str| Event::MessageStarted {
                message: message.into(),
                role: Role::Assistant,
            };
            voice.observe(&started("m"));
            voice.observe(&started("m"));
            assert_eq!(voice.speaking.len(), 1);
            // Another message of the same turn continues its reply.
            voice.observe(&started("n"));
            assert_eq!(voice.reply, 1);
            let mut ignore = drop::<AudioChunk>;
            assert!(Mute.synthesize("", &mut ignore).is_ok());
            voice.observe(&Event::TextDelta {
                message: "other".into(),
                text: "Hi.".into(),
            });
            voice.observe(&Event::MessageCompleted {
                message: "other".into(),
            });
            voice.observe(&Event::Closed { reason: None });
            assert!(voice.speaking.is_empty());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(sentences: &[&str]) -> Vec<String> {
        sentences
            .iter()
            .map(|&sentence| sentence.to_owned())
            .collect()
    }

    /// A sentence of `length` characters.
    fn sentence(length: usize) -> String {
        let mut text = "a".repeat(length - 1);
        text.push('.');
        text
    }

    #[test]
    fn brief_speech_speaks_whole_blocks_while_they_fit_and_cuts_at_a_block_boundary() {
        let (mut spent, mut cut) = (0, false);
        let first = vec![sentence(100), sentence(100)];
        let second = vec![sentence(90)];
        let third = vec![sentence(20)];
        let spoken = brief(
            &[first.clone(), Vec::new(), second.clone(), third.clone()],
            &mut spent,
            &mut cut,
        );
        assert_eq!(spoken, [first, Vec::new(), second, Vec::new()]);
        assert!(cut);
        assert_eq!(spent, 290);
        // Once cut, nothing later in the reply is spoken.
        assert_eq!(
            brief(&[block(&["Short."])], &mut spent, &mut cut),
            [Vec::<String>::new()]
        );
    }

    #[test]
    fn brief_speech_cuts_a_long_first_block_at_a_sentence() {
        let (mut spent, mut cut) = (0, false);
        let long = vec![sentence(200), sentence(90), sentence(50)];
        let spoken = brief(std::slice::from_ref(&long), &mut spent, &mut cut);
        assert_eq!(spoken, [long[..2].to_vec()]);
        assert!(cut);

        // A first sentence longer than the whole budget is still spoken, alone.
        let (mut spent, mut cut) = (0, false);
        let huge = vec![sentence(BRIEF_CHARACTERS * 2), sentence(10)];
        let spoken = brief(std::slice::from_ref(&huge), &mut spent, &mut cut);
        assert_eq!(spoken, [huge[..1].to_vec()]);
        assert!(cut);
    }

    #[test]
    fn a_long_paragraph_after_a_short_lead_in_is_cut_at_a_sentence() {
        let (mut spent, mut cut) = (0, false);
        let lead = block(&["Here is what I found:"]);
        let paragraph = vec![sentence(150), sentence(100), sentence(100)];
        let spoken = brief(&[lead.clone(), paragraph.clone()], &mut spent, &mut cut);
        assert_eq!(spoken, [lead, paragraph[..2].to_vec()]);
        assert!(cut);
    }

    #[test]
    fn the_limit_counts_characters_not_bytes() {
        let (mut spent, mut cut) = (0, false);
        // 120 characters of three bytes each.
        let wide = vec!["\u{3042}".repeat(120)];
        let spoken = brief(&[wide.clone(), wide.clone()], &mut spent, &mut cut);
        assert_eq!(spoken, [wide.clone(), wide]);
        assert!(!cut);
    }

    #[test]
    fn a_short_reply_is_spoken_in_full_and_not_cut() {
        let (mut spent, mut cut) = (0, false);
        let reply = [block(&["Done.", "All tests pass."]), block(&["Next?"])];
        assert_eq!(brief(&reply, &mut spent, &mut cut), reply);
        assert!(!cut);
    }
}
