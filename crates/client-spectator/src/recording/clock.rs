// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The recording's frame clock: the video has a constant rate, and every frame in it is timestamped by the recording's clock, which starts when the recording does and which the audio is timed by too, so that the two stay in step however long a frame took to draw. See `design/spectator.md`.
//!
//! The policy is to keep time rather than frames. Frame *n* of the file is at *n* / *fps* seconds, and it shows the newest frame drawn at or before then. A frame drawn late, or none drawn in a slot, repeats the one before: the file is never shorter than the recording, nor does it speed up after a slow frame. A frame drawn when its slot already has a newer one is dropped.
use std::{collections::VecDeque, time::Duration};

const NANOSECONDS: u128 = 1_000_000_000;

/// A constant frame rate, and where its frames are in time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameClock {
    fps: u32,
}

impl FrameClock {
    /// A clock of `fps` frames a second, which is not 0.
    pub fn new(fps: u32) -> Self {
        assert!(fps > 0, "a frame clock needs a rate");
        Self { fps }
    }

    /// How long a frame lasts.
    pub fn interval(self) -> Duration {
        self.timestamp(1)
    }

    /// When frame `index` is shown: `index / fps`, rounded up to a nanosecond, so that a frame is due exactly when its time has come, and no frame's time drifts from its neighbours'.
    pub fn timestamp(self, index: u64) -> Duration {
        let nanoseconds = (u128::from(index) * NANOSECONDS).div_ceil(u128::from(self.fps));
        Duration::from_nanos(nanoseconds as u64)
    }

    /// How many frames are due by `elapsed`, counting the frame at 0: those whose time has come.
    pub fn due(self, elapsed: Duration) -> u64 {
        (elapsed.as_nanos() * u128::from(self.fps) / NANOSECONDS) as u64 + 1
    }

    /// How many frames a recording of `elapsed` holds: the nearest whole number of frames to its length, and one at least, so that a recording stopped at once is still a video.
    pub fn frames_in(self, elapsed: Duration) -> u64 {
        let rounded = (elapsed.as_nanos() * u128::from(self.fps) + NANOSECONDS / 2) / NANOSECONDS;
        (rounded as u64).max(1)
    }
}

/// What a [`Pacer`] has done with the frames it was given, for the report at the end.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    /// Frames written to the file.
    pub written: u64,
    /// Of those, frames that repeated the one before, because nothing newer had been drawn by their time.
    pub duplicated: u64,
    /// Frames drawn that no slot wanted, because a newer one was drawn before the slot's time.
    pub superseded: u64,
}

/// Puts frames drawn at any times into the video's slots, one for each frame of the clock.
pub struct Pacer<F> {
    clock: FrameClock,
    /// The frames drawn and not yet used, oldest first, each with the recording time it was drawn at.
    drawn: VecDeque<(Duration, F)>,
    /// The frame in the slot before, which a slot with nothing newer repeats.
    held: Option<F>,
    /// The frame of a slot that could not be written, to write again.
    retry: Option<(F, bool)>,
    next: u64,
    counts: Counts,
}

impl<F: Clone> Pacer<F> {
    pub fn new(clock: FrameClock) -> Self {
        Self {
            clock,
            drawn: VecDeque::new(),
            held: None,
            retry: None,
            next: 0,
            counts: Counts::default(),
        }
    }

    /// Take a frame drawn at `at` on the recording's clock. Frames come in the order they were drawn.
    pub fn draw(&mut self, at: Duration, frame: F) {
        self.drawn.push_back((at, frame));
    }

    /// The slots written so far.
    pub fn counts(&self) -> Counts {
        self.counts
    }

    /// Whether the next slot has a frame to show: one drawn, now or before.
    pub fn ready(&self) -> bool {
        self.held.is_some() || self.retry.is_some() || !self.drawn.is_empty()
    }

    /// How many slots have been written, which is the number of the next.
    pub fn next(&self) -> u64 {
        self.next
    }

    /// The frame that fills the next slot, and whether it repeats the one before, or `None` while nothing has been drawn. Before the first frame is drawn, slots wait for it, and the first slots then show it, since the video starts with the recording.
    fn fill(&mut self) -> Option<(F, bool)> {
        let time = self.clock.timestamp(self.next);
        let mut newest = None;
        while self.drawn.front().is_some_and(|(at, _)| *at <= time) {
            let (_, frame) = self.drawn.pop_front().expect("a frame was just seen");
            if newest.replace(frame).is_some() {
                self.counts.superseded += 1;
            }
        }
        match newest {
            Some(frame) => Some((frame, false)),
            None if self.held.is_some() => self.held.clone().map(|frame| (frame, true)),
            // The first frame was drawn after this slot's time: it fills the slots before it.
            None => self.drawn.pop_front().map(|(_, frame)| (frame, false)),
        }
    }

    /// Write the slots from the next up to, but not including, `slots`, as `write` takes them, in order. Stops at the first slot that `write` fails, which is not counted and is written again by the next call with the frame it had, and at the first with no frame to show.
    pub fn write<E>(
        &mut self,
        slots: u64,
        mut write: impl FnMut(&F) -> Result<(), E>,
    ) -> Result<(), E> {
        while self.next < slots {
            let Some((frame, repeated)) = self.retry.take().or_else(|| self.fill()) else {
                break;
            };
            if let Err(error) = write(&frame) {
                self.retry = Some((frame, repeated));
                return Err(error);
            }
            self.held = Some(frame);
            self.counts.written += 1;
            self.counts.duplicated += u64::from(repeated);
            self.next += 1;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn millis(milliseconds: u64) -> Duration {
        Duration::from_millis(milliseconds)
    }

    #[test]
    fn frames_are_at_a_constant_rate_without_drifting() {
        for fps in [24, 25, 30, 60, 120] {
            let clock = FrameClock::new(fps);
            // After a minute of frames, the last is where a rate of exactly `fps` puts it.
            let frames = u64::from(fps) * 60;
            assert_eq!(clock.timestamp(frames), Duration::from_secs(60));
            assert_eq!(clock.timestamp(0), Duration::ZERO);
            assert!(clock.timestamp(1) > Duration::ZERO);
        }
        let clock = FrameClock::new(30);
        assert_eq!(clock.interval(), Duration::from_nanos(33_333_334));
        // 30 frames take a second: the interval's rounding does not accumulate in the timestamps.
        assert_eq!(clock.timestamp(30), Duration::from_secs(1));
        assert_eq!(clock.timestamp(1), Duration::from_nanos(33_333_334));
        assert_eq!(clock.timestamp(2), Duration::from_nanos(66_666_667));
    }

    #[test]
    fn a_frame_is_due_once_its_time_has_come_and_a_recording_holds_the_nearest_frames() {
        let clock = FrameClock::new(30);
        assert_eq!(clock.due(Duration::ZERO), 1);
        assert_eq!(clock.due(millis(33)), 1);
        assert_eq!(clock.due(Duration::from_nanos(33_333_333)), 1);
        assert_eq!(clock.due(Duration::from_nanos(33_333_334)), 2);
        // A frame is due exactly when its time has come, at any rate.
        for fps in [24, 25, 30, 48, 60, 90, 120] {
            let clock = FrameClock::new(fps);
            for index in [1, 2, 3, 7, 59, 1_001] {
                let time = clock.timestamp(index);
                assert_eq!(clock.due(time), index + 1, "{fps} {index}");
                assert_eq!(
                    clock.due(time - Duration::from_nanos(1)),
                    index,
                    "{fps} {index}"
                );
            }
        }
        assert_eq!(clock.due(Duration::from_secs(1)), 31);
        // A recording of a second holds 30 frames, a little over a second too; and never none.
        assert_eq!(clock.frames_in(Duration::from_secs(1)), 30);
        assert_eq!(clock.frames_in(millis(1_010)), 30);
        assert_eq!(clock.frames_in(millis(1_020)), 31);
        assert_eq!(clock.frames_in(Duration::ZERO), 1);
        assert_eq!(clock.frames_in(millis(5)), 1);
        assert_eq!(FrameClock::new(60).frames_in(Duration::from_secs(10)), 600);
    }

    #[test]
    #[should_panic(expected = "needs a rate")]
    fn a_clock_needs_a_rate() {
        FrameClock::new(0);
    }

    fn written(pacer: &mut Pacer<char>, slots: u64) -> Vec<char> {
        let mut frames = Vec::new();
        pacer
            .write(slots, |frame| {
                frames.push(*frame);
                Ok::<_, ()>(())
            })
            .unwrap();
        frames
    }

    #[test]
    fn frames_drawn_on_time_fill_a_slot_each() {
        let mut pacer = Pacer::new(FrameClock::new(10));
        for (index, frame) in "abcd".chars().enumerate() {
            pacer.draw(millis(index as u64 * 100), frame);
        }
        assert_eq!(written(&mut pacer, 4), ['a', 'b', 'c', 'd']);
        assert_eq!(
            pacer.counts(),
            Counts {
                written: 4,
                duplicated: 0,
                superseded: 0
            }
        );
        assert_eq!(pacer.next(), 4);
    }

    #[test]
    fn a_slow_frame_is_repeated_rather_than_dropping_time() {
        let mut pacer = Pacer::new(FrameClock::new(10));
        pacer.draw(Duration::ZERO, 'a');
        pacer.draw(millis(100), 'b');
        // 'c' took until 330 ms: slots 2 and 3 have nothing newer than 'b', and slot 3 is at 300 ms.
        pacer.draw(millis(330), 'c');
        pacer.draw(millis(400), 'd');
        assert_eq!(
            written(&mut pacer, 5),
            ['a', 'b', 'b', 'b', 'd'],
            "two repeats, and 'c' superseded by 'd' within its slot"
        );
        let counts = pacer.counts();
        assert_eq!(counts.written, 5);
        assert_eq!(counts.duplicated, 2);
        assert_eq!(counts.superseded, 1);
        // The file is as long as the time: five frames at ten a second.
        assert_eq!(FrameClock::new(10).timestamp(counts.written), millis(500));
    }

    #[test]
    fn frames_drawn_faster_than_the_rate_supersede_each_other_within_a_slot() {
        let mut pacer = Pacer::new(FrameClock::new(10));
        for (index, frame) in "abcdef".chars().enumerate() {
            pacer.draw(millis(index as u64 * 40), frame);
        }
        // Slots at 0, 100 and 200 ms take the newest drawn by then: a, c (at 80 ms) and f (at 200 ms).
        assert_eq!(written(&mut pacer, 3), ['a', 'c', 'f']);
        assert_eq!(pacer.counts().superseded, 3);
    }

    #[test]
    fn the_first_frame_fills_the_slots_before_it_and_slots_wait_for_it() {
        let mut pacer = Pacer::new(FrameClock::new(10));
        assert!(!pacer.ready());
        assert_eq!(written(&mut pacer, 3), [] as [char; 0]);
        assert_eq!(pacer.next(), 0);
        pacer.draw(millis(250), 'a');
        assert!(pacer.ready());
        // The first frame is shown from the first slot, as the video starts with the recording, but is not a repeat.
        assert_eq!(written(&mut pacer, 2), ['a', 'a']);
        assert_eq!(pacer.counts().duplicated, 1);
        assert_eq!(pacer.counts().written, 2);
        // Slots with nothing drawn since repeat it.
        assert_eq!(written(&mut pacer, 4), ['a', 'a']);
        assert_eq!(pacer.counts().duplicated, 3);
    }

    #[test]
    fn a_write_that_fails_is_not_counted_and_is_tried_again() {
        let mut pacer = Pacer::new(FrameClock::new(10));
        pacer.draw(Duration::ZERO, 'a');
        pacer.draw(millis(100), 'b');
        assert_eq!(pacer.write(2, |_| Err("broken pipe")), Err("broken pipe"));
        assert_eq!(pacer.counts(), Counts::default());
        assert_eq!(pacer.next(), 0);
        // The frame that failed is shown again, and not taken for a repeat.
        assert_eq!(written(&mut pacer, 2), ['a', 'b']);
        assert_eq!(pacer.counts().written, 2);
        assert_eq!(pacer.counts().duplicated, 0);
    }
}
