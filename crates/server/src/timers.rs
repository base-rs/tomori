// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! A space's timers: scheduled notes to its agent, each on a schedule and a condition of presence, kept with the
//! space. Pure: the time, the zone and who is present are given, and firings are returned for the space's event queue
//! to deliver. See `design/timers.md`.
use std::collections::VecDeque;

use jiff::Timestamp;
use jiff::tz::TimeZone;
use serde::{Deserialize, Serialize};

pub mod schedule;

use schedule::{Days, Schedule, instant, minutes};

/// The most timers a space keeps.
pub const MOST: usize = 8;

/// The fewest minutes between two occurrences of a timer.
pub const LEAST_MINUTES: u32 = 5;

/// The most minutes between two occurrences of a timer that fires every so many minutes: a week.
pub const LONGEST_EVERY: u32 = 7 * 1440;

/// The fewest minutes between two occurrences of a timer that fires whether or not anyone is present.
pub const LEAST_ALWAYS_MINUTES: u32 = 15;

/// The most firings a space delivers in any 24 hours, of all its timers.
pub const DAILY_FIRINGS: usize = 96;

/// The most firings a space delivers in any 24 hours while nobody is present, of timers that fire always.
pub const UNATTENDED_FIRINGS: usize = 24;

/// The longest a timer's note is, in characters.
pub const NOTE_CHARS: usize = 200;

/// The longest a timer's name is, in characters.
pub const NAME_CHARS: usize = 40;

/// The longest a timer's zone is, in bytes, longer than any IANA name.
pub const ZONE_BYTES: usize = 64;

/// How far ahead a one-off timer, or a timer's end, may be: a year.
pub const FURTHEST_DAYS: i64 = 366;

/// How long after the last participant stopped being present a space still counts as attended, so that lifting a
/// headset for a moment or reconnecting neither skips a firing nor counts as arriving again.
pub const GRACE_SECONDS: i64 = 120;

/// How long a timer looks just fired, in seconds.
pub const FIRED_SECONDS: i64 = 60;

/// The most occurrences a catch-up counts; more are said as at least this many.
const COUNTED: u32 = 999;

/// The version of the format timers are kept in.
pub const VERSION: u32 = 1;

/// Whether a timer fires only while someone is present.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Condition {
    /// At every occurrence, whether or not anyone is present.
    Always,
    /// At an occurrence while someone is present; skipped, not saved for later, while nobody is.
    Present,
    /// At an occurrence while someone is present, or, while nobody is, once as the first participant becomes present
    /// before the next occurrence.
    PresentOrArrival,
}

impl Condition {
    /// The condition `name` names, `always`, `present` or `present_or_arrival`, or `None`.
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "always" => Some(Self::Always),
            "present" | "when_present" => Some(Self::Present),
            "present_or_arrival" => Some(Self::PresentOrArrival),
            _ => None,
        }
    }

    /// The condition in short words.
    pub fn describe(self) -> &'static str {
        match self {
            Self::Always => "always",
            Self::Present => "when present",
            Self::PresentOrArrival => "when present or on arrival",
        }
    }
}

/// An occurrence a timer that fires on arrival waits with while nobody is present: when it fell, until when its period
/// lasts, and how many before it were missed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Waiting {
    pub due: i64,
    pub ends: i64,
    pub missed: u32,
}

/// One timer of a space.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Timer {
    pub name: String,
    /// What the agent set it for, in its own words, told back to it as data when it fires.
    pub note: String,
    pub schedule: Schedule,
    pub condition: Condition,
    /// The IANA name of the zone its times of day are in.
    pub zone: String,
    pub created: i64,
    /// No occurrence after this, in seconds since the Unix epoch, if set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<i64>,
    /// It ends once it has fired this many times, if set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<u32>,
    #[serde(default)]
    pub fired: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_fired: Option<i64>,
    /// Its next occurrence not yet looked at, or `None` once it has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waiting: Option<Waiting>,
    /// Occurrences skipped since it last fired, as nobody was present, or past a space's daily limit.
    #[serde(default)]
    pub skipped: u32,
    #[serde(default)]
    pub limited: u32,
}

/// One firing of a timer, for the space's event queue: the timer's name, which coalesces its firings, and the note's
/// text, without its brackets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Firing {
    pub name: String,
    pub text: String,
}

/// What the agent asked `set_timer` for, its arguments read but not yet checked.
#[derive(Clone, Debug, Default)]
pub struct Request {
    pub name: Option<String>,
    pub note: String,
    pub every_minutes: Option<u32>,
    pub daily_at: Option<String>,
    pub days: Option<Vec<String>>,
    pub once_at: Option<String>,
    pub in_minutes: Option<u32>,
    pub cron: Option<String>,
    pub condition: Option<Condition>,
    pub zone: Option<String>,
    pub until: Option<String>,
    pub count: Option<u32>,
}

/// The zone `name` names, an IANA name such as `Europe/London`, or why it cannot be found.
pub fn zone(name: &str) -> Result<TimeZone, String> {
    TimeZone::get(name).map_err(|_| {
        format!(
            "Tomori does not know the time zone {:?}: give an IANA name, such as Europe/London or Pacific/Auckland.",
            crate::space::one_line(name, ZONE_BYTES)
        )
    })
}

/// The zone this machine is in, by its IANA name, or `UTC` where it has none, as the default for timers.
pub fn system_zone() -> String {
    TimeZone::system().iana_name().unwrap_or("UTC").to_owned()
}

/// A date and time, `YYYY-MM-DD HH:MM` or `YYYY-MM-DDTHH:MM`, or a time of day, `HH:MM`, its next occurrence after
/// `now`, as the instant in `zone` it names, or `None`.
fn moment(text: &str, now: Timestamp, zone: &TimeZone) -> Option<Timestamp> {
    let text = text.trim();
    if let Some(time) = schedule::time_of_day(text) {
        let today = zone.to_datetime(now).date();
        let at = schedule::resolve(zone, today.to_datetime(time));
        return Some(if at > now {
            at
        } else {
            schedule::resolve(zone, today.tomorrow().unwrap_or(today).to_datetime(time))
        });
    }
    let (date, time) = text.split_once([' ', 'T'])?;
    let date: jiff::civil::Date = date.parse().ok()?;
    Some(schedule::resolve(
        zone,
        date.to_datetime(schedule::time_of_day(time)?),
    ))
}

/// A time as the agent reads it in `zone`: `HH:MM` today, or `Tue HH:MM` on another day.
fn clock(at: Timestamp, now: Timestamp, zone: &TimeZone) -> String {
    let (at, now) = (zone.to_datetime(at), zone.to_datetime(now));
    if at.date() == now.date() {
        at.strftime("%H:%M").to_string()
    } else {
        at.strftime("%a %H:%M").to_string()
    }
}

/// How long until `at` from `now`, in the largest whole unit, as `12 min` or `3 h`.
pub fn until(at: Timestamp, now: Timestamp) -> String {
    let seconds = (at.as_second() - now.as_second()).max(0);
    match seconds {
        0..60 => "under a minute".to_owned(),
        60..7200 => format!("{} min", seconds / 60),
        7200..172_800 => format!("{} h", seconds / 3600),
        _ => format!("{} days", seconds / 86_400),
    }
}

impl Timer {
    /// A timer from `request`, at `now`, its zone `default_zone` unless the request names one, numbered `number` for
    /// its name if it has none, or why it cannot be set.
    pub fn new(
        request: Request,
        now: Timestamp,
        default_zone: &str,
        number: u32,
    ) -> Result<Self, String> {
        let note = crate::space::one_line(&request.note, NOTE_CHARS + 1);
        if note.trim().is_empty() {
            return Err(
                "note says what the timer is for, which Tomori tells you when it fires.".to_owned(),
            );
        }
        if note.chars().count() > NOTE_CHARS {
            return Err(format!("note is at most {NOTE_CHARS} characters."));
        }
        let name = match request.name {
            Some(name) => {
                let name = crate::space::one_line(&name, NAME_CHARS + 1);
                if name.trim().is_empty() || name.chars().count() > NAME_CHARS {
                    return Err(format!(
                        "name is 1 to {NAME_CHARS} characters, on one line."
                    ));
                }
                name.trim().to_owned()
            }
            None => format!("timer {number}"),
        };
        let zone_name = request.zone.unwrap_or_else(|| default_zone.to_owned());
        let zone = zone(&zone_name)?;
        let condition = request.condition.unwrap_or(Condition::Present);
        let forms = [
            request.every_minutes.is_some(),
            request.daily_at.is_some(),
            request.once_at.is_some(),
            request.in_minutes.is_some(),
            request.cron.is_some(),
        ];
        let furthest = now + minutes(FURTHEST_DAYS * 1440);
        let schedule = match forms.iter().filter(|given| **given).count() {
            1 => {
                if let Some(every) = request.every_minutes {
                    if !(1..=LONGEST_EVERY).contains(&every) {
                        return Err(format!(
                            "every_minutes is from {LEAST_MINUTES} to {LONGEST_EVERY}, a week."
                        ));
                    }
                    Schedule::every(every, now)
                } else if let Some(at) = &request.daily_at {
                    let days = request.days.as_deref().unwrap_or_default();
                    let days = if days.is_empty() {
                        Days::EVERY
                    } else {
                        Days::parse(&days.iter().map(String::as_str).collect::<Vec<_>>())?
                    };
                    let time = schedule::time_of_day(at).ok_or(
                        "daily_at is a time of day, HH:MM on the 24-hour clock, such as 09:00.",
                    )?;
                    Schedule::Daily {
                        at: time.strftime("%H:%M").to_string(),
                        days,
                    }
                } else if let Some(at) = &request.once_at {
                    let at = moment(at, now, &zone).ok_or(
                        "once_at is a time of day, HH:MM, the next one, or a date and time, YYYY-MM-DD HH:MM.",
                    )?;
                    if at <= now || at > furthest {
                        return Err(format!(
                            "once_at is after now and within {FURTHEST_DAYS} days."
                        ));
                    }
                    Schedule::Once { at: at.as_second() }
                } else if let Some(after) = request.in_minutes {
                    if !(1..=FURTHEST_DAYS * 1440).contains(&i64::from(after)) {
                        return Err(format!("in_minutes is from 1 to {}.", FURTHEST_DAYS * 1440));
                    }
                    Schedule::Once {
                        at: (now + minutes(after.into())).as_second(),
                    }
                } else {
                    let cron = request.cron.as_deref().unwrap_or_default();
                    Schedule::Cron {
                        cron: schedule::Cron::parse(cron)?,
                    }
                }
            }
            _ => {
                return Err("Give exactly one schedule: every_minutes, daily_at (with days), once_at, in_minutes or cron.".to_owned());
            }
        };
        if request.days.is_some() && request.daily_at.is_none() {
            return Err("days goes with daily_at.".to_owned());
        }
        let least = match condition {
            Condition::Always => LEAST_ALWAYS_MINUTES,
            _ => LEAST_MINUTES,
        };
        if schedule.least_gap().is_some_and(|gap| gap < least) {
            return Err(format!(
                "A timer that fires {} fires at most every {least} minutes, its occurrences at least {least} minutes apart.",
                condition.describe()
            ));
        }
        let until = match &request.until {
            None => None,
            Some(until) => {
                let at = moment(until, now, &zone)
                    .ok_or("until is a time of day, HH:MM, the next one, or a date and time, YYYY-MM-DD HH:MM.")?;
                if at <= now || at > furthest {
                    return Err(format!(
                        "until is after now and within {FURTHEST_DAYS} days."
                    ));
                }
                Some(at.as_second())
            }
        };
        if request.count == Some(0) {
            return Err("count is at least 1.".to_owned());
        }
        let mut timer = Self {
            name,
            note,
            schedule,
            condition,
            zone: zone.iana_name().unwrap_or(&zone_name).to_owned(),
            created: now.as_second(),
            until,
            count: request.count,
            fired: 0,
            last_fired: None,
            next: None,
            waiting: None,
            skipped: 0,
            limited: 0,
        };
        timer.next = timer.after(now, &zone);
        if timer.next.is_none() {
            return Err("That schedule never fires before it ends.".to_owned());
        }
        Ok(timer)
    }

    fn time_zone(&self) -> TimeZone {
        TimeZone::get(&self.zone).unwrap_or(TimeZone::UTC)
    }

    /// Its next occurrence after `at`, within its end, if it has one.
    fn after(&self, at: Timestamp, zone: &TimeZone) -> Option<i64> {
        // A count is checked as it fires, which ends it.
        self.schedule
            .next_after(at, zone)
            .map(|next| next.as_second())
            .filter(|next| self.until.is_none_or(|until| *next <= until))
    }

    /// Whether it has nothing left to do: no occurrence ahead and none waiting for someone to arrive.
    pub fn finished(&self) -> bool {
        self.next.is_none() && self.waiting.is_none()
    }

    /// Its schedule and condition in short words, as `every hour, when present`.
    pub fn describe(&self) -> String {
        format!(
            "{}, {}",
            self.schedule.describe(&self.time_zone()),
            self.condition.describe()
        )
    }

    /// What it fired with at `now`: the occurrence `due`, how many were missed before it, whether it fired as someone
    /// arrived, and who is present.
    fn fire(
        &mut self,
        now: Timestamp,
        due: Timestamp,
        missed: u32,
        arrived: bool,
        present: &[String],
    ) -> Firing {
        let zone = self.time_zone();
        let mut said = vec![if arrived {
            format!(
                "fired {} as someone arrived, due {}",
                clock(now, now, &zone),
                clock(due, now, &zone)
            )
        } else {
            format!("fired {}", clock(due, now, &zone))
        }];
        if let Some(last) = self.last_fired {
            said.push(format!("last fired {}", clock(instant(last), now, &zone)));
        }
        if missed > 0 {
            let more = if missed >= COUNTED { "at least " } else { "" };
            said.push(format!(
                "missed {more}{missed} while Tomori could not tell you"
            ));
        }
        if self.skipped > 0 {
            said.push(format!("skipped {} while nobody was present", self.skipped));
        }
        if self.limited > 0 {
            said.push(format!(
                "held back {} by the space's daily limit",
                self.limited
            ));
        }
        said.push(format!(
            "present: {}",
            if present.is_empty() {
                "nobody".to_owned()
            } else {
                present.join(", ")
            }
        ));
        self.fired += 1;
        self.last_fired = Some(now.as_second());
        self.skipped = 0;
        self.limited = 0;
        self.waiting = None;
        if self.count.is_some_and(|count| self.fired >= count) {
            self.next = None;
        }
        if self.finished() {
            said.push("its last firing".to_owned());
        }
        said.push(format!("note: \"{}\"", self.note));
        Firing {
            name: self.name.clone(),
            text: format!(
                "Timer \"{}\" ({}): {}",
                self.name,
                self.describe(),
                said.join("; ")
            ),
        }
    }
}

/// How a timer looks to a participant now, for the space's state.
fn view(timer: &Timer, now: Timestamp, attended: bool) -> tomori_protocol::Timer {
    let state = if timer
        .last_fired
        .is_some_and(|last| now.as_second() - last < FIRED_SECONDS)
    {
        tomori_protocol::TimerState::Fired
    } else if timer.waiting.is_some() {
        tomori_protocol::TimerState::Awaiting
    } else if !attended && timer.condition != Condition::Always {
        tomori_protocol::TimerState::Paused
    } else {
        tomori_protocol::TimerState::Waiting
    };
    tomori_protocol::Timer::new(
        &timer.name,
        timer.schedule.describe(&timer.time_zone()),
        timer.condition.describe(),
        timer.next.map(|next| next as u64),
        state,
    )
}

/// What a space keeps of its timers across a restart, in a format of its own with its version.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Kept {
    pub version: u32,
    #[serde(default)]
    pub timers: Vec<Timer>,
    /// When the space's latest firings were delivered, for its daily limits, in seconds since the Unix epoch.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fired: Vec<i64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unattended: Vec<i64>,
    /// The number the next timer without a name is called by.
    #[serde(default)]
    pub numbered: u32,
}

/// A space's timers, and what their conditions and limits need to remember.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Timers {
    timers: Vec<Timer>,
    fired: VecDeque<i64>,
    unattended: VecDeque<i64>,
    numbered: u32,
    /// When a participant was last present, for the grace.
    last_present: Option<i64>,
    /// Whether the space counted as attended when last looked at, so that becoming attended is someone arriving.
    attended: bool,
    /// When they were last looked at, in seconds since the Unix epoch, which how they look is told as of.
    looked: i64,
    /// Restored after a restart and not looked at since.
    restored: bool,
    /// Until when, after a restart, nothing is looked at while nobody is present, in seconds since the Unix epoch.
    held_until: i64,
}

/// Whether two timers' names are the same, in any case, as Unicode lowercases them, and without spaces about them.
pub fn same_name(one: &str, other: &str) -> bool {
    one.trim().to_lowercase() == other.trim().to_lowercase()
}

impl Timers {
    /// The timers `kept` keeps, as [`Self::kept`] wrote it, read leniently, so that a record this server cannot read
    /// never stops the space, or any other, from loading: a version it does not read leaves every timer out, and a
    /// timer it cannot read, such as one of a schedule a newer server knows or with a field edited by hand, is left
    /// out alone, each with a warning naming `space`, and the timer. At most [`MOST`] are read. A catch-up after the
    /// restart waits for the grace, so that participants coming back count as present.
    pub fn restore(kept: &serde_json::Value, space: &str) -> Self {
        let mut restored = Self {
            restored: true,
            ..Self::default()
        };
        if kept.is_null() {
            return restored;
        }
        let version = kept.get("version").and_then(serde_json::Value::as_u64);
        if version != Some(u64::from(VERSION)) {
            log::warn!(
                "Space {space} keeps its timers in a version of their format this server does not read ({}); they are left out.",
                version.map_or_else(|| "none".to_owned(), |version| version.to_string())
            );
            return restored;
        }
        let listed = kept.get("timers").and_then(serde_json::Value::as_array);
        for value in listed.map(Vec::as_slice).unwrap_or_default() {
            let name = value
                .get("name")
                .and_then(serde_json::Value::as_str)
                .map_or_else(
                    || "without a name".to_owned(),
                    |name| format!("\"{}\"", crate::space::one_line(name, NAME_CHARS)),
                );
            match serde_json::from_value::<Timer>(value.clone()) {
                Ok(timer) if restored.timers.len() < MOST => {
                    if zone(&timer.zone).is_err() {
                        log::warn!(
                            "Space {space} keeps timer {name} in the time zone {:?}, which this server does not know; it goes by UTC.",
                            crate::space::one_line(&timer.zone, ZONE_BYTES)
                        );
                    }
                    restored.timers.push(timer);
                }
                Ok(_) => log::warn!(
                    "Space {space} keeps more than {MOST} timers; timer {name} is left out."
                ),
                Err(error) => log::warn!(
                    "Space {space} keeps timer {name}, which this server cannot read ({error}); it is left out."
                ),
            }
        }
        let seconds = |field: &str| -> VecDeque<i64> {
            kept.get(field)
                .and_then(|value| serde_json::from_value::<Vec<i64>>(value.clone()).ok())
                .unwrap_or_default()
                .into()
        };
        restored.fired = seconds("fired");
        restored.unattended = seconds("unattended");
        restored.numbered = kept
            .get("numbered")
            .and_then(serde_json::Value::as_u64)
            .map_or(0, |numbered| numbered.min(u64::from(u32::MAX)) as u32);
        restored
    }

    /// What is kept of them, as JSON in the format [`Kept`] describes, or null for nothing.
    pub fn kept(&self) -> serde_json::Value {
        if self.timers.is_empty() && self.numbered == 0 {
            return serde_json::Value::Null;
        }
        serde_json::to_value(Kept {
            version: VERSION,
            timers: self.timers.clone(),
            fired: self.fired.iter().copied().collect(),
            unattended: self.unattended.iter().copied().collect(),
            numbered: self.numbered,
        })
        .expect("timers serialize")
    }

    pub fn list(&self) -> &[Timer] {
        &self.timers
    }

    /// Set the timer `request` asks for at `now`, in `default_zone` unless it names one, replacing one of the same name:
    /// the timer, and whether it replaced one, or why it cannot be set.
    pub fn set(
        &mut self,
        request: Request,
        now: Timestamp,
        default_zone: &str,
    ) -> Result<(Timer, bool), String> {
        // A timer without a name takes the next number whose name no timer has, so that it never replaces one.
        let number = (self.numbered + 1..)
            .find(|number| {
                let name = format!("timer {number}");
                !self
                    .timers
                    .iter()
                    .any(|known| same_name(&known.name, &name))
            })
            .expect("fewer timers than numbers");
        let named = request.name.is_some();
        let timer = Timer::new(request, now, default_zone, number)?;
        let replacing = self
            .timers
            .iter()
            .position(|known| same_name(&known.name, &timer.name));
        match replacing {
            Some(index) => self.timers[index] = timer.clone(),
            None if self.timers.len() >= MOST => {
                return Err(format!(
                    "This space has {MOST} timers, the most it keeps: cancel one first, or set one of the same name to replace it."
                ));
            }
            None => self.timers.push(timer.clone()),
        }
        if !named {
            self.numbered = number;
        }
        Ok((timer, replacing.is_some()))
    }

    /// Cancel the timer called `name`, in any case: the timer, or `None` if there is none.
    pub fn cancel(&mut self, name: &str) -> Option<Timer> {
        let index = self
            .timers
            .iter()
            .position(|timer| same_name(&timer.name, name))?;
        Some(self.timers.remove(index))
    }

    /// Whether the space counts as attended at `now`: someone present, or someone present within the grace.
    pub fn attended(&self, now: Timestamp, present: &[String]) -> bool {
        !present.is_empty()
            || self
                .last_present
                .is_some_and(|last| now.as_second() - last <= GRACE_SECONDS)
    }

    /// Look at every timer at `now`, with `present` the participants present, and return what fires, within the
    /// space's daily limits. Occurrences since a timer was last looked at count as one, which fires at most once.
    pub fn tick(&mut self, now: Timestamp, present: &[String]) -> Vec<Firing> {
        let second = now.as_second();
        self.looked = second;
        // After a restart, nothing is looked at for the grace while nobody is present yet, so that a catch-up as the
        // space is bound again, before headsets reconnect, counts those who come back.
        if std::mem::take(&mut self.restored) {
            self.held_until = second + GRACE_SECONDS;
        }
        if second < self.held_until && present.is_empty() {
            return Vec::new();
        }
        let attended = self.attended(now, present);
        let arrived = !present.is_empty() && !self.attended;
        if !present.is_empty() {
            self.last_present = Some(second);
        }
        self.attended = attended;
        let day = second - 86_400;
        // Firings later than now, as after the clock was set back, count no more than those a day old.
        self.fired.retain(|at| *at > day && *at <= second);
        self.unattended.retain(|at| *at > day && *at <= second);
        let mut firings = Vec::new();
        for timer in &mut self.timers {
            let zone = timer.time_zone();
            // An occurrence waiting for someone to arrive lasts until the next occurrence, or its end.
            if let Some(waiting) = timer.waiting.clone() {
                if second >= waiting.ends {
                    timer.waiting = None;
                    timer.skipped += 1 + waiting.missed;
                } else if arrived {
                    if self.fired.len() < DAILY_FIRINGS {
                        self.fired.push_back(second);
                        firings.push(timer.fire(
                            now,
                            instant(waiting.due),
                            waiting.missed,
                            true,
                            present,
                        ));
                    } else {
                        timer.waiting = None;
                        timer.limited += 1;
                    }
                }
            }
            let Some(next) = timer.next.filter(|next| *next <= second) else {
                continue;
            };
            let (later, latest) = timer
                .schedule
                .occurrences(instant(next), now, &zone, COUNTED);
            let due = latest.unwrap_or(instant(next));
            // Any occurrence still waiting for an arrival lapsed above, as its period ends at this one.
            let missed = later;
            timer.next = timer.after(now, &zone);
            let fires = match timer.condition {
                Condition::Always => true,
                _ => attended,
            };
            if !fires {
                if timer.condition == Condition::PresentOrArrival {
                    let ends = timer.next.unwrap_or(due.as_second() + 86_400);
                    timer.waiting = Some(Waiting {
                        due: due.as_second(),
                        ends,
                        missed,
                    });
                } else {
                    timer.skipped += 1 + missed;
                }
                continue;
            }
            let unattended = !attended;
            if self.fired.len() >= DAILY_FIRINGS
                || (unattended && self.unattended.len() >= UNATTENDED_FIRINGS)
            {
                timer.limited += 1 + missed;
                continue;
            }
            self.fired.push_back(second);
            if unattended {
                self.unattended.push_back(second);
            }
            firings.push(timer.fire(now, due, missed, false, present));
        }
        self.timers.retain(|timer| !timer.finished());
        firings
    }

    /// How each timer looks as of when they were last looked at, for the space's state.
    pub fn views(&self) -> Vec<tomori_protocol::Timer> {
        let now = instant(self.looked);
        self.timers
            .iter()
            .map(|timer| view(timer, now, self.attended))
            .collect()
    }

    /// The timer called `name` described for the agent as a participant points at it, as of when they were last looked
    /// at, as `timer "status" (every hour, when present; next in 12 min)`, or `None` if there is none.
    pub fn describe(&self, name: &str) -> Option<String> {
        let now = instant(self.looked);
        let timer = self
            .timers
            .iter()
            .find(|timer| same_name(&timer.name, name))?;
        // A timer kept has an occurrence ahead or one waiting; one with neither is gone after the tick that ended it.
        let next = match timer.waiting {
            Some(_) => "waiting for someone to arrive".to_owned(),
            None => format!(
                "next in {}",
                until(instant(timer.next.unwrap_or(now.as_second())), now)
            ),
        };
        Some(format!(
            "timer \"{}\" ({}; {next})",
            crate::space::one_line(&timer.name, NAME_CHARS),
            timer.describe()
        ))
    }
}

#[cfg(test)]
mod tests;
