// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Whether agent speech arrives in time to play, so that a gap heard in it says which way it came. The server's pacer
//! sends each segment's frames at the rate they play, a little ahead, each stamped with when it plays, and playback
//! starts as a segment's first frames arrive; so a frame that arrives later after the first than its stamp says is due
//! after it should have played, and is heard as a gap, held up on the way, by the network or this device. A frame the
//! pacer itself sent late is stamped later, and the server says so instead. See `design/live-audio.md`.
use std::time::{Duration, Instant};

use tomori_protocol::{AGENT_SOURCE, AudioFrame};

/// How late a frame must arrive to be said: more than the jitter buffer's own start hides.
const LATE: Duration = Duration::from_millis(20);

/// How often lateness is said, at most, as recurring events are (see `design/client-logging.md`).
const REPORT: Duration = Duration::from_secs(30);

/// The frames of agent speech arriving one after another: when the first of them arrived, and its stamp, and the stamp
/// the next one bears, in microseconds.
#[derive(Clone, Copy, Debug)]
struct Run {
    arrived: Instant,
    stamp: u64,
    next: u64,
}

/// How agent speech has arrived on one connection.
#[derive(Debug, Default)]
pub(crate) struct Lateness {
    run: Option<Run>,
    /// When lateness was last said, the late frames since, and the furthest behind of them.
    said: Option<Instant>,
    count: u32,
    worst: Duration,
}

impl Lateness {
    /// `frame` arrived at `now`: if it is agent speech that arrived after it was due to play, say so, at most once every
    /// [`REPORT`], counting those in between. Returns what it said.
    pub fn arrived(&mut self, frame: &AudioFrame, now: Instant) -> Option<String> {
        if frame.source != AGENT_SOURCE || frame.sample_rate == 0 {
            return None;
        }
        let length = frame.samples.len() as u64 * 1_000_000 / u64::from(frame.sample_rate);
        let next = frame.timestamp.saturating_add(length);
        // A frame that follows on from the last plays as long after the first of them as its stamp says; any other starts
        // them afresh, after a breath or a stream that started again.
        let follows = self.run.filter(|run| run.next == frame.timestamp);
        let late = follows
            .and_then(|run| {
                let due =
                    run.arrived + Duration::from_micros(frame.timestamp.saturating_sub(run.stamp));
                now.checked_duration_since(due)
            })
            .filter(|late| *late >= LATE);
        self.run = Some(match (follows, late) {
            (Some(run), None) => Run { next, ..run },
            // Playback, run dry, starts again from a late frame, so those after it are measured from it.
            _ => Run {
                arrived: now,
                stamp: frame.timestamp,
                next,
            },
        });
        let late = late?;
        self.count += 1;
        self.worst = self.worst.max(late);
        if self
            .said
            .is_some_and(|said| now.saturating_duration_since(said) < REPORT)
        {
            return None;
        }
        let said = format!(
            "Agent speech arrived {} ms after it was due to play{}, so it was heard with a gap: the network or this device held it up",
            self.worst.as_millis(),
            match self.count {
                1 => String::new(),
                count => format!(", {count} times"),
            }
        );
        self.said = Some(now);
        self.count = 0;
        self.worst = Duration::ZERO;
        log::info!("{said}.");
        Some(said)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tomori_protocol::Gate;

    /// Frame `index` of agent speech at 24 kHz, stamped `offset` µs on from the first.
    fn speech(index: u64, offset: u64) -> AudioFrame {
        AudioFrame {
            gate: Gate::Live,
            source: AGENT_SOURCE,
            sequence: index as u32,
            timestamp: 5_000_000 + offset + index * 20_000,
            sample_rate: 24_000,
            samples: vec![0; 480],
        }
    }

    #[test]
    fn speech_that_arrives_after_it_was_due_to_play_is_said_at_most_every_thirty_seconds() {
        let start = Instant::now();
        let ms = |ms: u64| start + Duration::from_millis(ms);
        let mut lateness = Lateness::default();
        // Sent a fifth of a second ahead: a burst of ten frames, and then one every 20 ms, is in time.
        for index in 0..10 {
            assert_eq!(lateness.arrived(&speech(index, 0), start), None);
        }
        for index in 10..20 {
            assert_eq!(
                lateness.arrived(&speech(index, 0), ms((index - 10) * 20)),
                None
            );
        }
        // Held up for half a second, the next frame, due at 400 ms, arrives at 700 ms.
        assert_eq!(
            lateness.arrived(&speech(20, 0), ms(700)).as_deref(),
            Some(
                "Agent speech arrived 300 ms after it was due to play, so it was heard with a gap: the network or this device held it up"
            )
        );
        // Those after it are measured from it, as playback starts again from it.
        assert_eq!(lateness.arrived(&speech(21, 0), ms(700)), None);
        // Late again within thirty seconds, counted rather than said.
        assert_eq!(lateness.arrived(&speech(22, 0), ms(800)), None);
        assert_eq!(lateness.arrived(&speech(23, 0), ms(900)), None);
        assert_eq!(
            (lateness.count, lateness.worst),
            (2, Duration::from_millis(80))
        );
        // A breath, or a pacer that started again, stamps the next frame later: measured afresh, it is in time.
        assert_eq!(lateness.arrived(&speech(24, 400_000), ms(1_500)), None);
        // Thirty seconds on, the next late frame is said with those before it.
        assert_eq!(
            lateness
                .arrived(&speech(25, 400_000), ms(31_000))
                .as_deref(),
            Some(
                "Agent speech arrived 29480 ms after it was due to play, 3 times, so it was heard with a gap: the network or this device held it up"
            )
        );
        // Participants' audio, and frames without a rate, are not measured.
        let mut voice = speech(26, 400_000);
        voice.source = 3;
        assert_eq!(lateness.arrived(&voice, ms(40_000)), None);
        let mut empty = speech(26, 400_000);
        empty.sample_rate = 0;
        assert_eq!(lateness.arrived(&empty, ms(40_000)), None);
        assert!(
            lateness
                .run
                .is_some_and(|run| run.next == speech(26, 400_000).timestamp)
        );
    }
}
