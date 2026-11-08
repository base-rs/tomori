// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Explicit times, to the millisecond, as Prometheus keeps them.
use std::fmt;
use std::ops::Sub;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// A moment, in milliseconds since the Unix epoch, as Prometheus's timestamps are.
///
/// Every query names its time, so that several queries evaluated at one `Timestamp` agree with each other:
///
/// ```
/// use std::time::Duration;
/// use tomori_prometheus::Timestamp;
///
/// // The start of the current 5-second slot, so that a poll every 5 seconds fills one slot each.
/// let at = Timestamp::now().floor(Duration::from_secs(5));
/// assert_eq!(at.millis() % 5000, 0);
/// assert_eq!(Timestamp::from_millis(1_700_000_000_250).to_string(), "1700000000.250");
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp(i64);

impl Timestamp {
    /// `millis` milliseconds after the epoch, or before it if negative.
    pub const fn from_millis(millis: i64) -> Self {
        Self(millis)
    }

    /// Milliseconds since the epoch.
    pub const fn millis(self) -> i64 {
        self.0
    }

    /// Seconds since the epoch.
    pub fn seconds(self) -> f64 {
        self.0 as f64 / 1000.0
    }

    /// Now, by the system's clock.
    pub fn now() -> Self {
        let since = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        Self(i64::try_from(since.as_millis()).unwrap_or(i64::MAX))
    }

    /// The start of the `step`-long slot, counted from the epoch, that holds this moment; a `step` shorter than a millisecond is taken as one.
    pub fn floor(self, step: Duration) -> Self {
        let step = i64::try_from(step.as_millis()).unwrap_or(i64::MAX).max(1);
        Self(self.0.div_euclid(step) * step)
    }

    /// A timestamp from Prometheus's seconds, rounded to the millisecond, or `None` if they are not finite.
    pub(crate) fn from_seconds(seconds: f64) -> Option<Self> {
        seconds
            .is_finite()
            .then(|| Self((seconds * 1000.0).round() as i64))
    }
}

impl Sub<Duration> for Timestamp {
    type Output = Self;

    /// `duration` earlier, to the millisecond, stopping at the earliest timestamp.
    fn sub(self, duration: Duration) -> Self {
        let millis = i64::try_from(duration.as_millis()).unwrap_or(i64::MAX);
        Self(self.0.saturating_sub(millis))
    }
}

/// Seconds since the epoch with three decimals, as Prometheus's API reads a time.
impl fmt::Display for Timestamp {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sign = if self.0 < 0 { "-" } else { "" };
        let millis = self.0.unsigned_abs();
        write!(formatter, "{sign}{}.{:03}", millis / 1000, millis % 1000)
    }
}

/// The span and resolution of a range query: from `start` to `end`, both included, every `step`.
///
/// ```
/// use std::time::Duration;
/// use tomori_prometheus::{Range, Timestamp};
///
/// let end = Timestamp::now().floor(Duration::from_secs(15));
/// let range = Range::new(end - Duration::from_secs(3600), end, Duration::from_secs(15));
/// assert_eq!(range.points(), Some(241));
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Range {
    pub start: Timestamp,
    pub end: Timestamp,
    pub step: Duration,
}

impl Range {
    pub fn new(start: Timestamp, end: Timestamp, step: Duration) -> Self {
        Self { start, end, step }
    }

    /// How many points each series can have, or `None` when the range is empty or its step is shorter than a millisecond, which Prometheus would refuse.
    pub fn points(&self) -> Option<u64> {
        let step = u64::try_from(self.step.as_millis()).unwrap_or(u64::MAX);
        let span = u64::try_from(self.end.0.checked_sub(self.start.0)?).ok()?;
        (step > 0).then(|| span / step + 1)
    }
}

/// A duration as Prometheus reads one, in seconds with three decimals.
pub(crate) fn seconds(duration: Duration) -> String {
    format!("{}.{:03}", duration.as_secs(), duration.subsec_millis())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamps_are_milliseconds_written_as_seconds() {
        let at = Timestamp::from_millis(1_700_000_001_234);
        assert_eq!(at.millis(), 1_700_000_001_234);
        assert_eq!(at.seconds(), 1_700_000_001.234);
        assert_eq!(at.to_string(), "1700000001.234");
        assert_eq!(Timestamp::from_millis(-1_500).to_string(), "-1.500");
        assert_eq!(Timestamp::from_millis(5).to_string(), "0.005");
        assert_eq!(
            at.floor(Duration::from_secs(5)),
            Timestamp::from_millis(1_700_000_000_000)
        );
        assert_eq!(at.floor(Duration::ZERO), at);
        assert_eq!(
            Timestamp::from_millis(-1).floor(Duration::from_secs(1)),
            Timestamp::from_millis(-1000)
        );
        assert_eq!(
            at - Duration::from_millis(234),
            Timestamp::from_millis(1_700_000_001_000)
        );
        assert_eq!(
            Timestamp::from_millis(i64::MIN + 1) - Duration::from_secs(1),
            Timestamp::from_millis(i64::MIN)
        );
        assert!(Timestamp::now() > at);
        assert_eq!(
            Timestamp::from_seconds(1.2345),
            Some(Timestamp::from_millis(1235))
        );
        assert_eq!(Timestamp::from_seconds(f64::NAN), None);
        assert_eq!(seconds(Duration::from_millis(15_250)), "15.250");
    }

    #[test]
    fn a_range_counts_its_points() {
        let at = |millis| Timestamp::from_millis(millis);
        let second = Duration::from_secs(1);
        assert_eq!(Range::new(at(0), at(10_000), second).points(), Some(11));
        assert_eq!(Range::new(at(0), at(0), second).points(), Some(1));
        assert_eq!(Range::new(at(1), at(0), second).points(), None);
        assert_eq!(
            Range::new(at(0), at(10), Duration::from_micros(10)).points(),
            None
        );
        assert_eq!(
            Range::new(at(i64::MIN), at(i64::MAX), second).points(),
            None
        );
    }
}
