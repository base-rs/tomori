// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! When a timer's occurrences fall: every so many minutes or seconds, daily or on chosen weekdays at a time, once, to the
//! second, or as a small cron subset, each in the timer's time zone, with daylight saving's gaps and overlaps defined. Pure: the time and
//! zone are given. See `design/timers.md`.
use jiff::civil::{Date, DateTime, Time};
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp};
use serde::{Deserialize, Serialize};

/// The furthest ahead a daily, weekday or cron schedule is searched for its next occurrence: four years and a few
/// days, so that the 29th of February is found, and a schedule with none within it never fires.
const SEARCH_DAYS: usize = 4 * 366 + 7;

/// A second of Unix time as a timestamp, clamped to what a timestamp holds, so that a kept time out of range, as from
/// a hand-edited record, is the nearest it can be rather than a failure.
pub fn instant(second: i64) -> Timestamp {
    let (least, most) = (Timestamp::MIN.as_second(), Timestamp::MAX.as_second());
    Timestamp::from_second(second.clamp(least, most)).unwrap_or(Timestamp::UNIX_EPOCH)
}

/// The days of the week a daily schedule fires on, Monday as bit 0 to Sunday as bit 6.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Days(pub u8);

impl Days {
    pub const EVERY: Self = Self(0b111_1111);
    pub const WEEKDAYS: Self = Self(0b001_1111);
    pub const WEEKENDS: Self = Self(0b110_0000);

    /// The days `names` gives, each a day's name or its first three letters, or `weekdays`, `weekends` or `daily`, in
    /// any case, or why they cannot be read.
    pub fn parse(names: &[&str]) -> Result<Self, String> {
        const NAMES: [&str; 7] = [
            "monday",
            "tuesday",
            "wednesday",
            "thursday",
            "friday",
            "saturday",
            "sunday",
        ];
        let mut days = 0;
        for name in names {
            let name = name.trim().to_ascii_lowercase();
            days |= match name.as_str() {
                "daily" | "every day" => Self::EVERY.0,
                "weekdays" => Self::WEEKDAYS.0,
                "weekends" => Self::WEEKENDS.0,
                _ => NAMES
                    .iter()
                    .position(|day| name.len() >= 3 && day.starts_with(&name))
                    .map(|index| 1 << index)
                    .ok_or_else(|| {
                        format!(
                            "Tomori does not know the day {:?}: name days such as mon, tuesday, weekdays or weekends.",
                            crate::space::one_line(&name, 20)
                        )
                    })?,
            };
        }
        if days == 0 {
            return Err("days names no day.".to_owned());
        }
        Ok(Self(days))
    }

    fn has(self, date: Date) -> bool {
        self.0 & (1 << date.weekday().to_monday_zero_offset()) != 0
    }

    /// The days in short words: `daily`, `weekdays`, `weekends`, or their names, as `Mon, Wed, Fri`.
    pub fn describe(self) -> String {
        match self {
            Self::EVERY => "daily".to_owned(),
            Self::WEEKDAYS => "weekdays".to_owned(),
            Self::WEEKENDS => "weekends".to_owned(),
            Self(days) => ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]
                .iter()
                .enumerate()
                .filter(|(index, _)| days & (1 << index) != 0)
                .map(|(_, name)| *name)
                .collect::<Vec<_>>()
                .join(", "),
        }
    }
}

/// A five-field cron expression, minute, hour, day of the month, month and day of the week, of numbers, `*`, lists,
/// ranges and steps, kept with its text. Names of months and days, `L`, `W`, `#` and `?` are not taken. As in Vixie
/// cron, where either the day of the month or the day of the week begins with `*`, as `*` or `*/2` does, a day must
/// match both; otherwise a day matching either fires.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Cron {
    text: String,
    minutes: u64,
    hours: u32,
    days: u32,
    months: u16,
    weekdays: u8,
    any_day: bool,
    any_weekday: bool,
}

impl TryFrom<String> for Cron {
    type Error = String;

    fn try_from(text: String) -> Result<Self, String> {
        Self::parse(&text)
    }
}

impl From<Cron> for String {
    fn from(cron: Cron) -> Self {
        cron.text
    }
}

/// The values of one cron field, from `least` to `most`, as a bit set, or why it cannot be read.
fn field(text: &str, least: u32, most: u32, name: &str) -> Result<u64, String> {
    let bad = || {
        format!(
            "cron's {name} field, {:?}, is not numbers from {least} to {most}, *, lists, ranges or steps.",
            crate::space::one_line(text, 20)
        )
    };
    let mut set = 0u64;
    for part in text.split(',') {
        let (range, step) = match part.split_once('/') {
            Some((range, step)) => (range, step.parse::<u32>().map_err(|_| bad())?),
            None => (part, 1),
        };
        let (from, to) = match range {
            "*" => (least, most),
            _ => match range.split_once('-') {
                Some((from, to)) => (
                    from.parse::<u32>().map_err(|_| bad())?,
                    to.parse::<u32>().map_err(|_| bad())?,
                ),
                // A single value with a step runs to the field's end, as in `5/15`.
                None => {
                    let from = range.parse::<u32>().map_err(|_| bad())?;
                    (from, if part.contains('/') { most } else { from })
                }
            },
        };
        if step == 0 || from < least || to > most || from > to {
            return Err(bad());
        }
        for value in (from..=to).step_by(step as usize) {
            set |= 1 << value;
        }
    }
    Ok(set)
}

impl Cron {
    /// The cron expression `text`, or why it cannot be read.
    pub fn parse(text: &str) -> Result<Self, String> {
        let fields: Vec<&str> = text.split_whitespace().collect();
        let [minute, hour, day, month, weekday] = fields[..] else {
            return Err(
                "cron has five fields: minute, hour, day of the month, month and day of the week, as 0 9 * * 1-5."
                    .to_owned(),
            );
        };
        // Sunday is 0 or 7.
        let weekdays = field(weekday, 0, 7, "day of the week")?;
        Ok(Self {
            text: fields.join(" "),
            minutes: field(minute, 0, 59, "minute")?,
            hours: field(hour, 0, 23, "hour")? as u32,
            days: field(day, 1, 31, "day of the month")? as u32,
            months: field(month, 1, 12, "month")? as u16,
            weekdays: ((weekdays | (weekdays >> 7)) & 0x7f) as u8,
            // As Vixie cron marks a field beginning with `*`, a step of all of it such as `*/2` included.
            any_day: day.starts_with('*'),
            any_weekday: weekday.starts_with('*'),
        })
    }

    /// Its text, its fields separated by single spaces.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The fewest minutes between two of its times within a day, counting from the last of an hour to the first of the
    /// next, so that a schedule closer than the least interval is refused whatever hours it runs in.
    pub fn least_gap(&self) -> u32 {
        let minutes: Vec<u32> = (0..60)
            .filter(|minute| self.minutes & (1 << minute) != 0)
            .collect();
        let across = 60 - minutes[minutes.len() - 1] + minutes[0];
        minutes
            .windows(2)
            .map(|pair| pair[1] - pair[0])
            .fold(across, u32::min)
    }

    fn matches_day(&self, date: Date) -> bool {
        let month = self.months & (1 << date.month()) != 0;
        let day = self.days & (1 << date.day()) != 0;
        // Sunday is 0 here, as in cron.
        let weekday = self.weekdays & (1 << date.weekday().to_sunday_zero_offset()) != 0;
        // As in Vixie cron: where either day field begins with `*`, as `*` or `*/2`, a day must match both, so that
        // `*/2` keeps to odd days; where both are restricted otherwise, a day matching either fires.
        month
            && if self.any_day || self.any_weekday {
                day && weekday
            } else {
                day || weekday
            }
    }

    fn times(&self) -> impl Iterator<Item = Time> + '_ {
        (0..24i8)
            .filter(|hour| self.hours & (1 << hour) != 0)
            .flat_map(move |hour| {
                (0..60i8)
                    .filter(|minute| self.minutes & (1 << minute) != 0)
                    .map(move |minute| Time::constant(hour, minute, 0, 0))
            })
    }
}

/// When a timer's occurrences fall. Times of day are in the timer's own zone.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Schedule {
    /// Every `minutes`: where they divide a day, at each multiple of them after the zone's midnight, by its clock, and
    /// otherwise counted in elapsed time from `from`, in seconds since the Unix epoch, when it was set.
    Every { minutes: u32, from: i64 },
    /// Every `seconds`, counted in elapsed time from `from`, in seconds since the Unix epoch, when it was set, never on
    /// the clock, as for intervals in a workout.
    EverySeconds { seconds: u32, from: i64 },
    /// At `at`, `HH:MM`, on `days`.
    Daily { at: String, days: Days },
    /// Once, at `at`, in seconds since the Unix epoch.
    Once { at: i64 },
    /// As a cron expression gives.
    Cron { cron: Cron },
}

/// `HH:MM`, or `H:MM`, on the 24-hour clock, as a time of day, or `None`.
pub fn time_of_day(text: &str) -> Option<Time> {
    let (hour, minute) = text.trim().split_once(':')?;
    let digits = |part: &str, most: usize| {
        (1..=most).contains(&part.len()) && part.bytes().all(|byte| byte.is_ascii_digit())
    };
    if !digits(hour, 2) || !digits(minute, 2) || minute.len() != 2 {
        return None;
    }
    // At most two digits each, so each reads as a number.
    let (hour, minute) = (
        hour.parse::<i8>().unwrap_or(-1),
        minute.parse::<i8>().unwrap_or(-1),
    );
    ((0..24).contains(&hour) && minute < 60).then(|| Time::constant(hour, minute, 0, 0))
}

/// `HH:MM`, as [`time_of_day`] reads it, or `HH:MM:SS`, to the second, as a time of day, or `None`.
pub fn time_of_day_to_second(text: &str) -> Option<Time> {
    let text = text.trim();
    let Some((minutes, second)) = text
        .rsplit_once(':')
        .filter(|(minutes, _)| minutes.contains(':'))
    else {
        return time_of_day(text);
    };
    if second.len() != 2 || !second.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let second = second.parse::<i8>().ok().filter(|second| *second < 60)?;
    let time = time_of_day(minutes)?;
    Some(Time::constant(time.hour(), time.minute(), second, 0))
}

/// The instant the civil time `at` names in `zone`: in a gap, as when clocks go forward, as far past the gap's start as
/// `at` is, as 02:30 is 03:30 where 02:00 becomes 03:00; in an overlap, as when clocks go back, the earlier.
pub fn resolve(zone: &TimeZone, at: DateTime) -> Timestamp {
    zone.to_ambiguous_timestamp(at)
        .compatible()
        .unwrap_or(Timestamp::MAX)
}

impl Schedule {
    /// Every `minutes`, set at `now`: where they divide a day, on the clock, so that every hour falls on the hour, every
    /// two hours on the even hours and every day at midnight, through daylight saving; otherwise from `now`.
    pub fn every(minutes: u32, now: Timestamp) -> Self {
        Self::Every {
            minutes,
            from: now.as_second(),
        }
    }

    /// The first occurrence strictly after `after`, in `zone`, if there is one.
    pub fn next_after(&self, after: Timestamp, zone: &TimeZone) -> Option<Timestamp> {
        match self {
            // On the clock, as times of day are: a time in a gap moves past it, and one twice in an overlap is the first.
            // A kept record of none, as edited by hand, is every minute rather than a division by zero.
            Self::Every { minutes, .. } if 1440 % (*minutes).max(1) == 0 => {
                let minutes = (*minutes).max(1);
                let times = || {
                    (0..1440 / minutes).map(|index| {
                        let minute = index * minutes;
                        Time::constant((minute / 60) as i8, (minute % 60) as i8, 0, 0)
                    })
                };
                next_on_days(after, zone, |_| true, times)
            }
            Self::Every { minutes, from } => Some(elapsed(i64::from(*minutes) * 60, *from, after)),
            // A kept record of none, as edited by hand, is every second rather than a division by zero.
            Self::EverySeconds { seconds, from } => {
                Some(elapsed(i64::from(*seconds).max(1), *from, after))
            }
            Self::Once { at } => Some(instant(*at)).filter(|at| *at > after),
            Self::Daily { at, days } => {
                let time = time_of_day(at).unwrap_or(Time::midnight());
                next_on_days(after, zone, |date| days.has(date), || [time].into_iter())
            }
            Self::Cron { cron } => {
                next_on_days(after, zone, |date| cron.matches_day(date), || cron.times())
            }
        }
    }

    /// How many occurrences fall within `(after, until]`, at most `most`, and the latest of them.
    pub fn occurrences(
        &self,
        after: Timestamp,
        until: Timestamp,
        zone: &TimeZone,
        most: u32,
    ) -> (u32, Option<Timestamp>) {
        let (mut count, mut latest, mut at) = (0, None, after);
        while count < most
            && let Some(next) = self.next_after(at, zone).filter(|next| *next <= until)
        {
            count += 1;
            latest = Some(next);
            at = next;
        }
        (count, latest)
    }

    /// The schedule in short words, as `every hour`, `weekdays at 09:00`, `once at 14:30` or `cron 0 9 * * 1-5`.
    pub fn describe(&self, zone: &TimeZone) -> String {
        match self {
            Self::Every { minutes: 60, .. } => "every hour".to_owned(),
            Self::Every { minutes, .. } if minutes % 60 == 0 => {
                format!("every {} hours", minutes / 60)
            }
            Self::Every { minutes, .. } => format!("every {minutes} min"),
            Self::EverySeconds { seconds, .. } if seconds % 60 == 0 => {
                format!("every {} min", seconds / 60)
            }
            Self::EverySeconds { seconds, .. } => format!("every {seconds} s"),
            Self::Daily { at, days } => format!("{} at {at}", days.describe()),
            Self::Once { at } => format!(
                "once at {}",
                zone.to_datetime(instant(*at))
                    .strftime(if self.to_the_second(zone) {
                        "%H:%M:%S"
                    } else {
                        "%H:%M"
                    })
            ),
            Self::Cron { cron } => format!("cron {}", cron.text()),
        }
    }

    /// The fewest seconds between two of its occurrences, as far as its form says: `None` for a single occurrence.
    pub fn least_gap(&self) -> Option<u32> {
        match self {
            Self::Every { minutes, .. } => Some(minutes.saturating_mul(60)),
            Self::EverySeconds { seconds, .. } => Some(*seconds),
            Self::Daily { .. } => Some(86_400),
            Self::Once { .. } => None,
            Self::Cron { cron } => Some(cron.least_gap() * 60),
        }
    }

    /// Whether its times are said to the second, in `zone`: those of a schedule every so many seconds, and of a single
    /// occurrence that does not fall on a minute, as one set some seconds from now; the rest fall on minutes.
    pub fn to_the_second(&self, zone: &TimeZone) -> bool {
        match self {
            Self::EverySeconds { .. } => true,
            Self::Once { at } => zone.to_datetime(instant(*at)).second() != 0,
            _ => false,
        }
    }
}

/// The first of the occurrences every `period` seconds from `from` strictly after `after`.
fn elapsed(period: i64, from: i64, after: Timestamp) -> Timestamp {
    let elapsed = after.as_second() - from;
    let periods = if elapsed < 0 { 0 } else { elapsed / period + 1 };
    instant(from.saturating_add(periods.saturating_mul(period)))
}

/// The first instant strictly after `after` at one of `times` on a day `day` accepts, in `zone`. Each day's times are
/// resolved as [`resolve`] does; since a time in a gap is moved past it, a later time of day can fall before an earlier
/// one resolved, so the earliest of a day's and the next day's is taken.
fn next_on_days<I: Iterator<Item = Time>>(
    after: Timestamp,
    zone: &TimeZone,
    day: impl Fn(Date) -> bool,
    times: impl Fn() -> I,
) -> Option<Timestamp> {
    let mut date = zone.to_datetime(after).date();
    let mut found: Option<(Timestamp, usize)> = None;
    for index in 0..SEARCH_DAYS {
        if found.is_some_and(|(_, at)| index > at + 1) {
            break;
        }
        if day(date) {
            for time in times() {
                let at = resolve(zone, date.to_datetime(time));
                if at > after && found.is_none_or(|(best, _)| at < best) {
                    found = Some((at, found.map_or(index, |(_, day)| day)));
                }
            }
        }
        // The last date a timestamp holds stays the last, and matches no more than it did.
        date = date.tomorrow().unwrap_or(date);
    }
    found.map(|(at, _)| at)
}

/// A span of minutes, as a duration.
pub fn minutes(minutes: i64) -> SignedDuration {
    SignedDuration::from_mins(minutes)
}
