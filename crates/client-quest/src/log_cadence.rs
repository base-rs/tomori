// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Logs events that repeat often at a slow cadence, so that they cannot push lifecycle events out of the headset's logcat buffer. See `design/client-logging.md`.

use std::time::{Duration, Instant};

/// How often the headset summarizes the stereo frames it submits.
pub const FRAME_SUMMARY: Duration = Duration::from_secs(60);

/// Lets the first occurrence of an event be logged, then at most one per interval, counting those skipped in between.
#[derive(Debug)]
pub struct Cadence {
    interval: Duration,
    last: Option<Instant>,
    skipped: u64,
}

/// An occurrence that is due to be logged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Due {
    /// Occurrences not logged since the previous logged one.
    pub skipped: u64,
    /// Time since the previous logged occurrence, or `None` for the first.
    pub elapsed: Option<Duration>,
}

impl Cadence {
    pub const fn new(interval: Duration) -> Self {
        Self {
            interval,
            last: None,
            skipped: 0,
        }
    }

    /// Records an occurrence at `now`, returning whether to log it.
    pub fn record(&mut self, now: Instant) -> Option<Due> {
        let elapsed = self.last.map(|last| now.saturating_duration_since(last));
        if elapsed.is_some_and(|elapsed| elapsed < self.interval) {
            self.skipped += 1;
            return None;
        }
        self.last = Some(now);
        Some(Due {
            skipped: std::mem::take(&mut self.skipped),
            elapsed,
        })
    }
}

impl Due {
    /// Occurrences per second since the previous logged one, including this one.
    pub fn rate(&self) -> Option<f64> {
        self.elapsed
            .filter(|elapsed| !elapsed.is_zero())
            .map(|elapsed| (self.skipped + 1) as f64 / elapsed.as_secs_f64())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINUTE: Duration = Duration::from_secs(60);

    #[test]
    fn logs_the_first_occurrence() {
        let mut cadence = Cadence::new(MINUTE);
        assert_eq!(
            cadence.record(Instant::now()),
            Some(Due {
                skipped: 0,
                elapsed: None
            })
        );
    }

    #[test]
    fn skips_occurrences_within_the_interval_and_counts_them() {
        let start = Instant::now();
        let mut cadence = Cadence::new(MINUTE);
        cadence.record(start);
        for frame in 1..90 * 60 {
            let now = start + Duration::from_secs_f64(f64::from(frame) / 90.0);
            assert_eq!(cadence.record(now), None, "frame {frame}");
        }
        let due = cadence.record(start + MINUTE).unwrap();
        assert_eq!(
            due,
            Due {
                skipped: 90 * 60 - 1,
                elapsed: Some(MINUTE)
            }
        );
        assert!((due.rate().unwrap() - 90.0).abs() < 1e-9);
    }

    #[test]
    fn starts_counting_again_after_logging() {
        let start = Instant::now();
        let mut cadence = Cadence::new(MINUTE);
        cadence.record(start);
        cadence.record(start + Duration::from_secs(1));
        cadence.record(start + Duration::from_secs(2));
        assert_eq!(cadence.record(start + MINUTE).unwrap().skipped, 2);
        cadence.record(start + MINUTE + Duration::from_secs(1));
        // The next interval runs from the occurrence last logged, not from the first.
        assert_eq!(
            cadence.record(start + MINUTE * 2 - Duration::from_secs(1)),
            None
        );
        assert_eq!(
            cadence.record(start + MINUTE * 2),
            Some(Due {
                skipped: 2,
                elapsed: Some(MINUTE)
            })
        );
    }

    #[test]
    fn logs_an_occurrence_after_a_long_pause_at_once() {
        let start = Instant::now();
        let mut cadence = Cadence::new(MINUTE);
        cadence.record(start);
        let due = cadence.record(start + MINUTE * 10).unwrap();
        assert_eq!(due.skipped, 0);
        assert_eq!(due.rate(), Some(1.0 / 600.0));
    }

    #[test]
    fn has_no_rate_for_the_first_occurrence() {
        let mut cadence = Cadence::new(Duration::ZERO);
        let now = Instant::now();
        assert_eq!(cadence.record(now).unwrap().rate(), None);
        // A zero interval logs every occurrence, and two at the same instant have no rate.
        assert_eq!(cadence.record(now).unwrap().rate(), None);
    }
}
