// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Agent speech kept for the participants who missed it, and played to them from the start of the latest reply. See `design/speech.md` and `design/live-audio.md`.
//!
//! A space's [`Retention`] records every item of its live agent speech stream, the announcements, frames and breaths the pacer sends, bounded to the latest [`RETAINED_REPLIES`] replies and [`RETAINED_AUDIO`] of audio. A reply is the speech of one agent turn. Each participant in the space, or who was in it, is a listener:
//!
//! - **Live** listeners hear live speech, and have heard everything recorded so far.
//! - **Unreported** listeners hear live speech too, but their client has not yet said whether it is listening. A client that never reports engagement, such as an older client or the spectator, stays at the live edge, as before.
//! - **Away** listeners are in the space but not listening, and **absent** ones have left it. Each keeps the position up to which it heard.
//! - **Streaming** listeners hear retained speech from their own cursor, at the rate it plays, from [`protocol::REPLAY_SOURCE`], and are not sent live speech meanwhile, so that nobody hears the same reply twice at once.
//!
//! A participant who joins or returns while listening, having missed part of the latest reply, streams from that reply's start and then everything after it, until their cursor reaches the live edge, where they become live. A replay streams the same way, from the start of the reply asked for, but stops as soon as a later reply starts live: live speech wins.
//!
//! Nothing here touches the network or a clock; the hub records items, applies joins, departures and engagement, and asks [`Retention::pace`] what each streaming listener should be sent at a given time.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use protocol::{AudioFrame, Engagement, Gate, REPLAY_SOURCE, ServerMessage};

/// The most audio retained per space, about 14 MB of 24 kHz mono speech. The oldest is dropped first.
pub const RETAINED_AUDIO: Duration = Duration::from_secs(300);

/// The most replies whose speech is retained per space: the latest, and the one before it, so that a listener still catching up on one reply when the next starts keeps hearing it.
pub const RETAINED_REPLIES: usize = 2;

/// How far ahead of its playback time a frame is sent, to cover network jitter. Kept well below the client's jitter buffer limit.
pub const LEAD: Duration = Duration::from_millis(100);

/// A breath between spoken segments, so that a list or a series of paragraphs does not run together.
pub const SEGMENT_GAP: Duration = Duration::from_millis(400);

/// One item of a space's agent speech stream.
#[derive(Clone, Debug)]
pub enum Item {
    /// A [`ServerMessage::Speech`] announcing the frames that follow.
    Announce(ServerMessage),
    Frame {
        sample_rate: u32,
        samples: Arc<[i16]>,
    },
    /// A breath of [`SEGMENT_GAP`] between two segments.
    Gap,
}

impl Item {
    fn duration(&self) -> Duration {
        match self {
            Item::Frame {
                sample_rate,
                samples,
            } if *sample_rate > 0 => {
                Duration::from_secs_f64(samples.len() as f64 / f64::from(*sample_rate))
            }
            _ => Duration::ZERO,
        }
    }
}

/// Something to send one streaming listener.
#[derive(Debug, PartialEq)]
pub enum Delivery {
    Message(ServerMessage),
    Frame(AudioFrame),
}

struct Entry {
    reply: u64,
    item: Item,
}

#[derive(Debug, Clone, PartialEq)]
enum State {
    Live,
    /// Heard everything before `heard`, then joined when the stream ended at `since`, hearing live speech from there while its client has not said whether it is listening.
    Unreported {
        heard: u64,
        since: u64,
    },
    /// In the space but not listening, having heard everything before `heard`.
    Away {
        heard: u64,
    },
    /// Not in the space, having heard everything before `heard`.
    Absent {
        heard: u64,
    },
    Streaming(Stream),
}

#[derive(Debug, Clone, PartialEq)]
struct Stream {
    cursor: u64,
    /// When the next frame should start playing, once one has been sent.
    clock: Option<Instant>,
    /// For a replay, the newest reply when it started: live speech of a later reply stops it. A catch-up continues through later replies.
    until: Option<u64>,
}

struct Listener {
    state: State,
    /// The next sequence number of this listener's [`REPLAY_SOURCE`] frames, continued across streams so that its client's jitter buffer sees one source.
    sequence: u32,
    /// What this listener's [`REPLAY_SOURCE`] frame timestamps count from, kept across streams like the sequence.
    epoch: Instant,
}

/// The retained speech of one space, and where each of its listeners is in it.
#[derive(Default)]
pub struct Retention {
    entries: VecDeque<Entry>,
    /// The index of the first retained entry. Indices count every entry ever recorded.
    first: u64,
    audio: Duration,
    /// The reply number and first entry index of each retained reply, oldest first. A reply's start may have been dropped.
    replies: VecDeque<(u64, u64)>,
    listeners: HashMap<String, Listener>,
    /// Set once the space has closed: nothing more is kept, and nothing is played again.
    closed: bool,
}

impl Retention {
    /// The index of the next entry to be recorded: the live edge.
    fn end(&self) -> u64 {
        self.first + self.entries.len() as u64
    }

    /// Where the latest retained reply begins, or its oldest retained entry if its start was dropped.
    fn latest_start(&self) -> Option<u64> {
        let (_, start) = self.replies.back()?;
        (self.end() > *start).then(|| (*start).max(self.first))
    }

    /// Whether `participant` is sent live speech: everyone not streaming, away or absent, including anyone not known yet.
    pub fn hears_live(&self, participant: &str) -> bool {
        self.listeners
            .get(participant)
            .is_none_or(|listener| matches!(listener.state, State::Live | State::Unreported { .. }))
    }

    /// Whether `participant` is hearing retained speech rather than live speech.
    #[cfg(test)]
    pub fn is_streaming(&self, participant: &str) -> bool {
        self.listeners
            .get(participant)
            .is_some_and(|listener| matches!(listener.state, State::Streaming(_)))
    }

    /// The audio retained, for tests of the bounds.
    #[cfg(test)]
    fn retained(&self) -> (Duration, usize) {
        (self.audio, self.replies.len())
    }

    /// Record one item of live speech, part of reply number `reply`. A replay of an earlier reply stops when a later one starts, and its listener hears the new reply live from its start.
    pub fn record(&mut self, reply: u64, item: Item) {
        if self.closed {
            return;
        }
        if self
            .replies
            .back()
            .is_none_or(|(latest, _)| *latest != reply)
        {
            let start = self.end();
            self.replies.push_back((reply, start));
            for listener in self.listeners.values_mut() {
                if let State::Streaming(stream) = &listener.state
                    && stream.until.is_some_and(|until| until != reply)
                {
                    listener.state = State::Live;
                }
            }
        }
        self.audio += item.duration();
        self.entries.push_back(Entry { reply, item });
        self.trim();
    }

    /// Drop the oldest entries beyond the bounds, and forget absent listeners who missed everything still retained.
    fn trim(&mut self) {
        while let Some(front) = self.entries.front() {
            let excess_replies = self.replies.len() > RETAINED_REPLIES
                && self
                    .replies
                    .front()
                    .is_some_and(|(reply, _)| *reply == front.reply);
            if !excess_replies && self.audio <= RETAINED_AUDIO {
                break;
            }
            let entry = self.entries.pop_front().expect("checked above");
            self.audio = self.audio.saturating_sub(entry.item.duration());
            self.first += 1;
            // A reply none of whose entries remain is no longer retained.
            while self.replies.len() > 1
                && self
                    .replies
                    .get(1)
                    .is_some_and(|(_, start)| *start <= self.first)
            {
                self.replies.pop_front();
            }
            if self.entries.is_empty() {
                self.replies.clear();
            }
        }
        let first = self.first;
        self.listeners.retain(|_, listener| match listener.state {
            State::Absent { heard } => heard > first,
            _ => true,
        });
    }

    /// Where `participant` had heard up to, before being here now.
    fn heard(&self, participant: &str) -> u64 {
        match self
            .listeners
            .get(participant)
            .map(|listener| &listener.state)
        {
            Some(State::Absent { heard } | State::Away { heard }) => *heard,
            Some(State::Unreported { heard, .. }) => *heard,
            Some(State::Streaming(stream)) => stream.cursor,
            Some(State::Live) => self.end(),
            None => 0,
        }
    }

    fn set(&mut self, participant: &str, state: State) {
        self.listeners
            .entry(participant.to_owned())
            .or_insert_with(|| Listener {
                state: State::Live,
                sequence: 0,
                epoch: Instant::now(),
            })
            .state = state;
    }

    /// The state of a listener who missed what was recorded from `heard` up to `until`: streaming from the start of the latest reply if they missed any of it, and live otherwise.
    fn resume(&self, heard: u64, until: u64) -> State {
        match self.latest_start() {
            Some(start) if heard < until && until > start => State::Streaming(Stream {
                cursor: start,
                clock: None,
                until: None,
            }),
            _ => State::Live,
        }
    }

    /// `participant` joined the space, having last reported `engagement` on their connection, if anything. Returns whether they now stream, so that the pacer must be woken.
    pub fn joined(&mut self, participant: &str, engagement: Option<Engagement>) -> bool {
        let heard = self.heard(participant);
        let state = match engagement {
            None => State::Unreported {
                heard,
                since: self.end(),
            },
            Some(engagement) if !engagement.is_listening() => State::Away { heard },
            Some(_) => self.resume(heard, self.end()),
        };
        let streaming = matches!(state, State::Streaming(_));
        self.set(participant, state);
        streaming
    }

    /// `participant` left the space, or their connection ended.
    pub fn left(&mut self, participant: &str) {
        let heard = match self
            .listeners
            .get(participant)
            .map(|listener| &listener.state)
        {
            None | Some(State::Absent { .. }) => return,
            // Hearing live speech, they heard everything recorded so far.
            Some(State::Live | State::Unreported { .. }) => self.end(),
            Some(State::Streaming(stream)) => stream.cursor,
            Some(State::Away { heard }) => *heard,
        };
        self.set(participant, State::Absent { heard });
    }

    /// `participant`, who is in the space, reported `engagement`. Returns whether they now stream.
    pub fn engagement(&mut self, participant: &str, engagement: Engagement) -> bool {
        let current = self
            .listeners
            .get(participant)
            .map(|listener| listener.state.clone());
        let state = if engagement.is_listening() {
            match current {
                Some(State::Unreported { heard, since }) => self.resume(heard, since),
                Some(State::Away { heard } | State::Absent { heard }) => {
                    self.resume(heard, self.end())
                }
                None => self.resume(0, self.end()),
                Some(State::Live | State::Streaming(_)) => return false,
            }
        } else {
            match current {
                Some(State::Away { .. }) => return false,
                Some(State::Absent { heard }) => State::Away { heard },
                Some(State::Streaming(stream)) => State::Away {
                    heard: stream.cursor,
                },
                Some(State::Unreported { heard, since }) => State::Away {
                    // Live speech heard since joining counts only if nothing was missed before it.
                    heard: if heard < since { heard } else { self.end() },
                },
                Some(State::Live) | None => State::Away { heard: self.end() },
            }
        };
        let streaming = matches!(state, State::Streaming(_));
        self.set(participant, state);
        streaming
    }

    /// Play the reply with an announcement for `message`, or the latest reply, again to `participant` from its start. Refused while they are away, as nothing is played to them then.
    pub fn replay(&mut self, participant: &str, message: Option<&str>) -> Result<(), &'static str> {
        if self.closed {
            return Err("The agent session has stopped, so its replies are no longer kept.");
        }
        if self.listeners.get(participant).is_some_and(|listener| {
            matches!(listener.state, State::Away { .. } | State::Absent { .. })
        }) {
            return Err("Nothing is played to you while you are away.");
        }
        let start = match message {
            None => self
                .latest_start()
                .ok_or("There is no spoken reply to play again yet.")?,
            Some(message) => self
                .entries
                .iter()
                .position(|entry| {
                    matches!(&entry.item, Item::Announce(ServerMessage::Speech { message: announced, .. }) if announced == message)
                })
                .map(|index| self.first + index as u64)
                .ok_or("That reply is no longer kept, so it cannot be played again.")?,
        };
        let newest = self.replies.back().map(|(reply, _)| *reply);
        self.set(
            participant,
            State::Streaming(Stream {
                cursor: start,
                clock: None,
                until: newest,
            }),
        );
        Ok(())
    }

    /// Stop `participant`'s replay or catch-up: they hear live speech from here on.
    pub fn stop(&mut self, participant: &str) {
        if self
            .listeners
            .get(participant)
            .is_some_and(|listener| matches!(listener.state, State::Streaming(_)))
        {
            self.set(participant, State::Live);
        }
    }

    /// Forget everything kept and every listener, and keep nothing more, as when the space closes. Everyone hears what is still spoken live.
    pub fn close(&mut self) {
        *self = Self {
            closed: true,
            ..Self::default()
        };
    }

    /// Collect what each streaming listener should be sent by `now`, in order. Returns when the next is due, if any listener is still streaming. A listener whose cursor reaches the live edge becomes live.
    pub fn pace(
        &mut self,
        now: Instant,
        deliveries: &mut Vec<(String, Delivery)>,
    ) -> Option<Instant> {
        let (first, end) = (self.first, self.end());
        let mut next: Option<Instant> = None;
        for (participant, listener) in &mut self.listeners {
            let State::Streaming(stream) = &mut listener.state else {
                continue;
            };
            loop {
                // A listener so far behind that their next entry was dropped skips to the oldest retained one.
                stream.cursor = stream.cursor.max(first);
                if stream.cursor >= end {
                    listener.state = State::Live;
                    break;
                }
                let entry = &self.entries[(stream.cursor - first) as usize];
                match &entry.item {
                    Item::Announce(message) => {
                        let mut message = message.clone();
                        if let ServerMessage::Speech { replay, .. } = &mut message {
                            *replay = true;
                        }
                        deliveries.push((participant.clone(), Delivery::Message(message)));
                    }
                    Item::Gap => {
                        let clock = stream.clock.filter(|clock| *clock >= now).unwrap_or(now);
                        stream.clock = Some(clock + SEGMENT_GAP);
                    }
                    item @ Item::Frame {
                        sample_rate,
                        samples,
                    } => {
                        let clock = stream.clock.filter(|clock| *clock >= now).unwrap_or(now);
                        let due = clock.checked_sub(LEAD).unwrap_or(clock);
                        if due > now {
                            next = Some(next.map_or(due, |next| next.min(due)));
                            break;
                        }
                        deliveries.push((
                            participant.clone(),
                            Delivery::Frame(AudioFrame {
                                gate: Gate::Live,
                                source: REPLAY_SOURCE,
                                sequence: listener.sequence,
                                timestamp: clock
                                    .saturating_duration_since(listener.epoch)
                                    .as_micros() as u64,
                                sample_rate: *sample_rate,
                                samples: samples.to_vec(),
                            }),
                        ));
                        listener.sequence = listener.sequence.wrapping_add(1);
                        stream.clock = Some(clock + item.duration());
                    }
                }
                stream.cursor += 1;
            }
        }
        next
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 24_000;
    /// 20 ms at [`RATE`].
    const FRAME: usize = 480;

    fn announce(segment: u32, message: &str) -> Item {
        Item::Announce(ServerMessage::Speech {
            segment,
            message: message.to_owned(),
            text: format!("Segment {segment}."),
            sample_rate: RATE,
            replay: false,
        })
    }

    fn frame(value: i16) -> Item {
        Item::Frame {
            sample_rate: RATE,
            samples: vec![value; FRAME].into(),
        }
    }

    /// Record a reply of one segment of `frames` frames whose samples are `value`.
    fn speak(retention: &mut Retention, reply: u64, message: &str, value: i16, frames: usize) {
        retention.record(reply, announce(reply as u32, message));
        for _ in 0..frames {
            retention.record(reply, frame(value));
        }
        retention.record(reply, Item::Gap);
    }

    /// Everything `participant` is sent by pacing until nothing is due, advancing a simulated clock: the announcements' texts and the sample value of each frame.
    fn drain(
        retention: &mut Retention,
        participant: &str,
        mut now: Instant,
    ) -> (Vec<String>, Vec<i16>, Instant) {
        let (mut texts, mut values) = (Vec::new(), Vec::new());
        loop {
            let mut deliveries = Vec::new();
            let next = retention.pace(now, &mut deliveries);
            for (to, delivery) in deliveries {
                assert_eq!(to, participant);
                match delivery {
                    Delivery::Message(ServerMessage::Speech { text, replay, .. }) => {
                        assert!(replay);
                        texts.push(text);
                    }
                    Delivery::Message(other) => panic!("unexpected {other:?}"),
                    Delivery::Frame(frame) => {
                        assert_eq!(frame.source, REPLAY_SOURCE);
                        values.push(frame.samples[0]);
                    }
                }
            }
            match next {
                Some(next) => now = next.max(now),
                None => return (texts, values, now),
            }
        }
    }

    #[test]
    fn a_participant_who_joins_after_a_reply_hears_it_from_its_start() {
        let now = Instant::now();
        let mut retention = Retention::default();
        speak(&mut retention, 1, "m1", 1, 5);
        assert!(retention.joined("ada", Some(Engagement::Engaged)));
        assert!(!retention.hears_live("ada"));
        let (texts, values, _) = drain(&mut retention, "ada", now);
        assert_eq!(texts, ["Segment 1."]);
        assert_eq!(values, [1; 5]);
        // Caught up, she follows live speech again, and does not hear the reply twice.
        assert!(retention.hears_live("ada"));
        retention.left("ada");
        assert!(!retention.joined("ada", Some(Engagement::Engaged)));
    }

    #[test]
    fn catch_up_is_paced_at_the_rate_it_plays() {
        let start = Instant::now();
        let mut retention = Retention::default();
        speak(&mut retention, 1, "m1", 1, 50);
        retention.joined("ada", Some(Engagement::Engaged));
        let (_, values, end) = drain(&mut retention, "ada", start);
        assert_eq!(values.len(), 50);
        // A second of audio is sent over about a second, only slightly ahead.
        let elapsed = end - start;
        assert!(elapsed >= Duration::from_millis(880), "{elapsed:?}");
        assert!(elapsed <= Duration::from_secs(1), "{elapsed:?}");
    }

    #[test]
    fn someone_who_heard_the_reply_live_is_not_played_it_again() {
        let mut retention = Retention::default();
        retention.joined("ada", Some(Engagement::Engaged));
        speak(&mut retention, 1, "m1", 1, 5);
        assert!(retention.hears_live("ada"));
        // She takes the headset off and puts it back on, and leaves and comes back.
        assert!(!retention.engagement("ada", Engagement::Away));
        assert!(!retention.engagement("ada", Engagement::Engaged));
        retention.left("ada");
        assert!(!retention.joined("ada", Some(Engagement::Engaged)));
        assert!(retention.hears_live("ada"));
    }

    #[test]
    fn returning_mid_reply_starts_from_the_reply_and_follows_it_without_overlap() {
        let now = Instant::now();
        let mut retention = Retention::default();
        retention.joined("ada", Some(Engagement::Engaged));
        retention.joined("bob", Some(Engagement::Engaged));
        speak(&mut retention, 1, "m1", 1, 3);
        // Ada takes the headset off part way through the second reply.
        retention.record(2, announce(2, "m2"));
        retention.record(2, frame(2));
        assert!(!retention.engagement("ada", Engagement::Away));
        assert!(!retention.hears_live("ada"));
        for _ in 0..20 {
            retention.record(2, frame(3));
        }
        // She puts it back on while the reply is still being spoken.
        assert!(retention.engagement("ada", Engagement::Engaged));
        assert!(!retention.hears_live("ada"));
        assert!(retention.hears_live("bob"));
        let mut deliveries = Vec::new();
        retention.pace(now, &mut deliveries);
        // The rest of the reply arrives live while she catches up; she is not sent it live.
        retention.record(2, frame(4));
        assert!(!retention.hears_live("ada"));
        let (texts, values, _) = drain(&mut retention, "ada", now);
        let mut all: Vec<i16> = deliveries
            .into_iter()
            .filter_map(|(_, delivery)| match delivery {
                Delivery::Frame(frame) => Some(frame.samples[0]),
                Delivery::Message(_) => None,
            })
            .collect();
        all.extend(values);
        let mut expected = vec![2];
        expected.extend([3; 20]);
        expected.push(4);
        assert_eq!(all, expected);
        assert!(texts.is_empty(), "the announcement came first");
        assert!(retention.hears_live("ada"));
    }

    #[test]
    fn a_client_that_reports_nothing_stays_live() {
        let now = Instant::now();
        let mut retention = Retention::default();
        speak(&mut retention, 1, "m1", 1, 3);
        // The spectator, or an older client, joins and never reports engagement.
        assert!(!retention.joined("spectator", None));
        assert!(retention.hears_live("spectator"));
        let mut deliveries = Vec::new();
        assert_eq!(retention.pace(now, &mut deliveries), None);
        assert!(deliveries.is_empty());
    }

    #[test]
    fn a_first_report_after_joining_catches_up_on_what_was_missed_before() {
        let now = Instant::now();
        let mut retention = Retention::default();
        speak(&mut retention, 1, "m1", 1, 3);
        retention.record(2, announce(2, "m2"));
        retention.record(2, frame(2));
        // The headset joins mid-reply, and reports that it is worn a moment later.
        assert!(!retention.joined("ada", None));
        retention.record(2, frame(3));
        assert!(retention.engagement("ada", Engagement::Engaged));
        // She missed the start of the latest reply, so she hears it from its start.
        let (texts, values, _) = drain(&mut retention, "ada", now);
        assert_eq!(texts, ["Segment 2."]);
        assert_eq!(values, [2, 3]);

        // Carol joins between replies and hears the next one live from its start, so her report changes nothing,
        // although she missed earlier replies.
        assert!(!retention.joined("carol", None));
        retention.record(3, announce(3, "m3"));
        retention.record(3, frame(4));
        assert!(!retention.engagement("carol", Engagement::Engaged));
        assert!(retention.hears_live("carol"));

        // Bob reconnects having heard everything, and his report changes nothing.
        retention.joined("bob", Some(Engagement::Engaged));
        let mut deliveries = Vec::new();
        retention.pace(now, &mut deliveries);
        retention.left("bob");
        assert!(!retention.joined("bob", None));
        assert!(!retention.engagement("bob", Engagement::Engaged));
        assert!(retention.hears_live("bob"));
    }

    #[test]
    fn a_listener_away_before_anything_was_said_stays_away_while_speech_is_kept() {
        let now = Instant::now();
        let mut retention = Retention::default();
        retention.joined("ada", None);
        retention.engagement("ada", Engagement::Away);
        speak(&mut retention, 1, "m1", 1, 3);
        speak(&mut retention, 2, "m2", 2, 3);
        speak(&mut retention, 3, "m3", 3, 3);
        assert!(!retention.hears_live("ada"));
        assert!(retention.engagement("ada", Engagement::Engaged));
        let (texts, _, _) = drain(&mut retention, "ada", now);
        assert_eq!(texts, ["Segment 3."]);
    }

    #[test]
    fn joining_away_waits_for_the_return() {
        let now = Instant::now();
        let mut retention = Retention::default();
        speak(&mut retention, 1, "m1", 1, 2);
        assert!(!retention.joined("ada", Some(Engagement::Away)));
        assert!(!retention.hears_live("ada"));
        // A system menu over the space still counts as listening.
        assert!(retention.engagement("ada", Engagement::Distracted));
        let (_, values, _) = drain(&mut retention, "ada", now);
        assert_eq!(values, [1, 1]);
    }

    #[test]
    fn a_replay_plays_the_latest_reply_to_one_listener_and_live_speech_stops_it() {
        let now = Instant::now();
        let mut retention = Retention::default();
        retention.joined("ada", Some(Engagement::Engaged));
        retention.joined("bob", Some(Engagement::Engaged));
        assert_eq!(
            retention.replay("ada", None),
            Err("There is no spoken reply to play again yet.")
        );
        speak(&mut retention, 1, "m1", 1, 2);
        speak(&mut retention, 2, "m2", 2, 50);
        retention.replay("ada", None).unwrap();
        assert!(!retention.hears_live("ada"));
        assert!(retention.hears_live("bob"));
        let mut deliveries = Vec::new();
        retention.pace(now, &mut deliveries);
        assert!(deliveries.iter().all(|(to, _)| to == "ada"));
        assert!(
            matches!(&deliveries[0].1, Delivery::Message(ServerMessage::Speech { message, .. }) if message == "m2")
        );
        // A new reply starts live part way through: Ada hears it live, from its start.
        retention.record(3, announce(3, "m3"));
        assert!(retention.hears_live("ada"));
        // An earlier reply can be asked for by its message, while it is kept.
        retention.replay("bob", Some("m2")).unwrap();
        assert!(retention.is_streaming("bob"));
        retention.stop("bob");
        assert!(retention.hears_live("bob"));
        assert_eq!(
            retention.replay("bob", Some("m1")),
            Err("That reply is no longer kept, so it cannot be played again.")
        );
    }

    #[test]
    fn a_replay_is_refused_while_away_and_the_return_still_catches_up() {
        let now = Instant::now();
        let mut retention = Retention::default();
        retention.joined("ada", Some(Engagement::Away));
        speak(&mut retention, 1, "m1", 1, 2);
        assert_eq!(
            retention.replay("ada", None),
            Err("Nothing is played to you while you are away.")
        );
        assert!(!retention.hears_live("ada"));
        assert!(retention.engagement("ada", Engagement::Engaged));
        let (_, values, _) = drain(&mut retention, "ada", now);
        assert_eq!(values, [1, 1]);
    }

    #[test]
    fn closing_forgets_what_was_kept_and_keeps_nothing_more() {
        let mut retention = Retention::default();
        retention.joined("ada", Some(Engagement::Away));
        speak(&mut retention, 1, "m1", 1, 2);
        retention.close();
        assert_eq!(retention.retained(), (Duration::ZERO, 0));
        assert!(retention.hears_live("ada"));
        // What is still spoken as the space closes is heard live, but not kept.
        speak(&mut retention, 2, "m2", 2, 2);
        assert_eq!(retention.retained(), (Duration::ZERO, 0));
        assert_eq!(
            retention.replay("ada", None),
            Err("The agent session has stopped, so its replies are no longer kept.")
        );
        assert!(!retention.joined("bob", Some(Engagement::Engaged)));
        assert!(retention.hears_live("bob"));
    }

    #[test]
    fn a_replay_while_the_reply_is_still_spoken_follows_it_to_its_end() {
        let now = Instant::now();
        let mut retention = Retention::default();
        retention.joined("ada", Some(Engagement::Engaged));
        retention.record(1, announce(1, "m1"));
        retention.record(1, frame(1));
        retention.replay("ada", None).unwrap();
        // More of the same reply is not a later reply, so the replay continues and Ada is not sent it live.
        retention.record(1, frame(2));
        assert!(!retention.hears_live("ada"));
        let (_, values, _) = drain(&mut retention, "ada", now);
        assert_eq!(values, [1, 2]);
    }

    #[test]
    fn retention_keeps_at_most_two_replies_and_five_minutes() {
        let now = Instant::now();
        let mut retention = Retention::default();
        speak(&mut retention, 1, "m1", 1, 10);
        speak(&mut retention, 2, "m2", 2, 10);
        speak(&mut retention, 3, "m3", 3, 10);
        assert_eq!(retention.retained(), (Duration::from_millis(400), 2));
        assert!(retention.replay("ada", Some("m1")).is_err());
        assert!(retention.replay("ada", Some("m2")).is_ok());

        // A single reply longer than the bound keeps only its latest five minutes.
        let frames = (RETAINED_AUDIO.as_millis() / 20) as usize + 100;
        speak(&mut retention, 4, "m4", 4, frames);
        let (audio, replies) = retention.retained();
        assert_eq!(audio, RETAINED_AUDIO);
        assert_eq!(replies, 1);
        // Catching up starts from the oldest audio still kept.
        retention.joined("bob", Some(Engagement::Engaged));
        let (texts, values, _) = drain(&mut retention, "bob", now);
        assert!(texts.is_empty(), "the reply's announcement was dropped");
        assert_eq!(values.len(), frames - 100);
    }

    #[test]
    fn a_listener_who_fell_behind_what_is_kept_skips_to_the_oldest() {
        let now = Instant::now();
        let mut retention = Retention::default();
        speak(&mut retention, 1, "m1", 1, 2);
        retention.joined("ada", Some(Engagement::Engaged));
        // Two more replies start before she has heard any of the first.
        speak(&mut retention, 2, "m2", 2, 2);
        speak(&mut retention, 3, "m3", 3, 2);
        let (texts, values, _) = drain(&mut retention, "ada", now);
        assert_eq!(texts, ["Segment 2.", "Segment 3."]);
        assert_eq!(values, [2, 2, 3, 3]);
    }

    #[test]
    fn replayed_frames_continue_one_sequence() {
        let now = Instant::now();
        let mut retention = Retention::default();
        retention.joined("ada", Some(Engagement::Engaged));
        speak(&mut retention, 1, "m1", 1, 3);
        let mut sequences = Vec::new();
        for _ in 0..2 {
            retention.replay("ada", None).unwrap();
            let mut clock = now;
            loop {
                let mut deliveries = Vec::new();
                let next = retention.pace(clock, &mut deliveries);
                for (_, delivery) in deliveries {
                    if let Delivery::Frame(frame) = delivery {
                        sequences.push(frame.sequence);
                    }
                }
                match next {
                    Some(next) => clock = next,
                    None => break,
                }
            }
        }
        assert_eq!(sequences, [0, 1, 2, 3, 4, 5]);
    }
}
