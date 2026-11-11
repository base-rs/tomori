// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Watching a space's workers for the agent: from what each was last seen doing, as its backend reports it, whether
//! it has failed, left a message unanswered or stalled, each told to the agent once until it recovers, in one note for
//! each condition, at most once every [`PACE`], and shown on the worker for everyone in the space. Pure: the time is
//! given. Nothing is kept across a restart. See `design/events.md`.
use std::collections::HashMap;

use tomori_backend::{RunningTool, WorkerActivity};
use tomori_protocol::{Worker, WorkerState};

use crate::limits;
use crate::space::{one_line, worker_finished};

/// How long a message given to a worker may go without progress before it is unanswered, in milliseconds.
pub const UNANSWERED: u64 = 5 * 60_000;

/// How long a worker may go without progress before it is stalled, in milliseconds.
pub const STALLED: u64 = 10 * 60_000;

/// How long a worker running a tool may go without other progress before it is stalled, in milliseconds: a long
/// command, such as a build or a test run, is not a hang.
pub const STALLED_RUNNING_TOOL: u64 = 30 * 60_000;

/// The least time between two deliveries of notes about stalled or unanswered workers, in milliseconds; a failure is
/// told at once.
pub const PACE: u64 = 2 * 60_000;

/// The most deliveries of notes about workers in a space in a day, and of those while nobody is in it.
pub const DAILY: usize = 48;
pub const UNATTENDED: usize = 12;

/// How many times a worker's threshold doubles at most, once for each episode of it already told, until it completes
/// cleanly.
const BACKOFF: u32 = 3;

/// How far the wall clock may move from the monotonic clock between two looks, in milliseconds, before Tomori takes it
/// for a sleep or a jump of the clock and moves the times it holds by as much, rather than counting it as time workers
/// spent without progress.
pub const JUMP: i64 = 60_000;

/// The most jumps of the clock remembered, to move times reported after them that were observed before.
const JUMPS: usize = 8;

const DAY: u64 = 24 * 60 * 60_000;

/// The most workers a note names; the rest are counted.
pub const NAMED: usize = 5;

/// The most characters of a worker's title, or of why it failed, a note gives.
const TITLE_CHARS: usize = 60;
const ERROR_CHARS: usize = 160;

/// The most workers whose activity a space keeps, as many as a backend tracks.
const MOST: usize = 4096;

/// What is wrong with a worker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Failed,
    Unanswered,
    Stalled,
}

impl Kind {
    /// The kind's name, as the `workers` tool gives it and a note's key.
    pub fn name(self) -> &'static str {
        match self {
            Self::Failed => "failed",
            Self::Unanswered => "unanswered",
            Self::Stalled => "stalled",
        }
    }
}

/// What is wrong with a worker, and the evidence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Condition {
    pub kind: Kind,
    /// Since when, in milliseconds since the Unix epoch: for an unanswered worker, when it was given the message; for a
    /// stalled one, when it last made progress; for a failed one, when it was last seen active.
    pub since: u64,
    /// For an unanswered worker, when the message was sent to it.
    pub sent: Option<u64>,
    /// Why it failed, as the agent service said, if it did and said.
    pub error: Option<String>,
    /// The tool it is running, if it is.
    pub tool: Option<RunningTool>,
    /// How its last step ended, as the agent said.
    pub finish: Option<String>,
}

/// What is wrong with `worker`, last seen doing `activity`, at `now`, if anything is, its thresholds multiplied by
/// `backoff`. A worker whose last progress is not known is not judged.
fn judge(worker: &Worker, activity: &WorkerActivity, now: u64, backoff: u64) -> Option<Condition> {
    if activity.progressed == 0 {
        return None;
    }
    let condition = |kind, since| Condition {
        kind,
        since,
        sent: None,
        error: activity.error.clone(),
        tool: activity.tool.clone(),
        finish: activity.finish.clone(),
    };
    if worker.state == WorkerState::Error {
        return Some(condition(Kind::Failed, activity.progressed));
    }
    let waiting = activity.waiting || worker.state == WorkerState::Waiting;
    if waiting {
        return None;
    }
    if let (Some(sent), Some(since)) = (activity.message, activity.unanswered_since) {
        // Given while a tool was running, as to a worker in a long build, it has as long as a stall.
        let threshold = match &activity.tool {
            Some(tool) if tool.since <= since => STALLED_RUNNING_TOOL,
            _ => UNANSWERED,
        };
        if now.saturating_sub(since) >= threshold * backoff {
            return Some(Condition {
                sent: Some(sent),
                ..condition(Kind::Unanswered, since)
            });
        }
    }
    let threshold = if activity.tool.is_some() {
        STALLED_RUNNING_TOOL
    } else {
        STALLED
    };
    // A message waiting, queued behind a run or not yet unanswered for long, does not stop a worker that has stopped.
    (!worker_finished(worker.state)
        && now.saturating_sub(activity.progressed) >= threshold * backoff)
        .then(|| Condition {
            sent: activity.message,
            ..condition(Kind::Stalled, activity.progressed)
        })
}

/// `ms` milliseconds as whole minutes, at least one.
fn minutes(ms: u64) -> String {
    format!("{} min", (ms / 60_000).max(1))
}

/// How a step that ended as `finish` is described.
fn ended(finish: &str) -> String {
    match finish {
        "tool-calls" => "with tool calls".to_owned(),
        "stop" => "normally".to_owned(),
        "length" => "at its length limit".to_owned(),
        "error" => "in an error".to_owned(),
        other => format!("as '{}'", one_line(other, TITLE_CHARS)),
    }
}

/// `items`, each named, as `A`, `A and B` or `A, B and C`, naming at most [`NAMED`] and counting the rest.
fn listed(items: &[String]) -> String {
    let named = &items[..items.len().min(NAMED)];
    let more = items.len() - named.len();
    let mut parts: Vec<String> = named.to_vec();
    if more > 0 {
        parts.push(format!("{more} more"));
    }
    match parts.split_last() {
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
        None => String::new(),
    }
}

/// A space's watch over its workers.
#[derive(Debug, Default)]
pub struct Watch {
    /// What each worker was last seen doing, as its backend reported it.
    activity: HashMap<String, WorkerActivity>,
    /// What is wrong with each worker, as last judged.
    conditions: HashMap<String, Condition>,
    /// The condition each worker was last told to the agent in, until it recovers.
    told: HashMap<String, Kind>,
    /// The workers in a condition not yet told, in the order they entered it.
    pending: Vec<String>,
    /// When notes about stalled or unanswered workers were last delivered.
    last: Option<u64>,
    /// When notes about workers were delivered in the last day, and whether anyone was in the space.
    delivered: std::collections::VecDeque<(u64, bool)>,
    /// How many episodes of each worker were told since it last completed cleanly, which its thresholds double by.
    episodes: HashMap<String, u32>,
    /// The time last given, in milliseconds since the Unix epoch, and the monotonic clock then, if given.
    now: u64,
    monotonic: Option<std::time::Instant>,
    /// The latest jumps of the clock, when each was noticed and by how much, in milliseconds.
    jumps: std::collections::VecDeque<(u64, i64)>,
    /// Until when notes are paused, having reached the most for a day, once the agent was told so.
    paused: Option<u64>,
}

/// `at`, in milliseconds since the Unix epoch, as a time of day in this machine's zone, such as `14:05`, with the day
/// where it is not today.
fn clock_time(at: u64) -> String {
    let zone = jiff::tz::TimeZone::system();
    let millis = i64::try_from(at).unwrap_or(i64::MAX);
    match jiff::Timestamp::from_millisecond(millis) {
        Ok(at) => at.to_zoned(zone).strftime("%a %H:%M").to_string(),
        Err(_) => "tomorrow".to_owned(),
    }
}

/// `at` moved by `shift` milliseconds.
fn moved(at: u64, shift: i64) -> u64 {
    at.saturating_add_signed(shift)
}

/// The times in `activity` observed before `jump`, those earlier than when it was noticed, moved by its shift, as if
/// the jump had not happened.
fn rebase(activity: &mut WorkerActivity, (noticed, shift): (u64, i64)) {
    let before = |at: &mut u64| {
        if *at != 0 && *at < noticed {
            *at = moved(*at, shift);
        }
    };
    before(&mut activity.progressed);
    for at in [&mut activity.message, &mut activity.unanswered_since]
        .into_iter()
        .flatten()
    {
        before(at);
    }
    if let Some(tool) = &mut activity.tool {
        before(&mut tool.since);
    }
}

impl Watch {
    /// Whether any worker has been seen active, and so may be judged.
    pub fn watching(&self) -> bool {
        !self.activity.is_empty()
    }

    /// The worker `id` was last seen doing `activity`. Times in it observed before a jump of the clock are moved as the
    /// jump moved those already held.
    pub fn observe(&mut self, id: String, mut activity: WorkerActivity) {
        if self.activity.len() < MOST || self.activity.contains_key(&id) {
            for jump in &self.jumps {
                rebase(&mut activity, *jump);
            }
            self.activity.insert(id, activity);
        }
    }

    /// What every worker is doing became unknown, as when the backend lost its connection: none is judged until it is
    /// reported again, and what was already told stays told.
    pub fn unknown(&mut self) {
        self.activity.clear();
        self.conditions.clear();
        self.pending.clear();
    }

    /// Forget the activity of workers the space no longer has, once its list is known to be every worker.
    pub fn retain(&mut self, workers: &[Worker]) {
        self.activity
            .retain(|id, _| workers.iter().any(|worker| &worker.id == id));
    }

    /// Judge `workers` again at `now`, ending the episode of each that recovered and noting each that entered a
    /// condition to be told. Returns whether any worker's condition, as shown, changed.
    ///
    /// With `monotonic`, the monotonic clock now, a wall clock that moved more than [`JUMP`] from it since the last look,
    /// as across a sleep, moves every time held by as much, so that the sleep does not count as time without progress.
    pub fn tick(
        &mut self,
        now: u64,
        monotonic: Option<std::time::Instant>,
        workers: &[Worker],
    ) -> bool {
        if let (Some(monotonic), Some(before)) = (monotonic, self.monotonic) {
            let wall = i64::try_from(now).unwrap_or(i64::MAX)
                - i64::try_from(self.now).unwrap_or(i64::MAX);
            let steady =
                i64::try_from(monotonic.duration_since(before).as_millis()).unwrap_or(i64::MAX);
            let shift = wall.saturating_sub(steady);
            if shift.abs() > JUMP {
                log::info!(
                    "The clock moved {shift} ms beyond the time that passed; worker times move with it."
                );
                self.jump(self.now, shift);
            }
            self.now = now;
        } else {
            self.now = self.now.max(now);
        }
        if monotonic.is_some() {
            self.monotonic = monotonic;
        }
        let now = self.now;
        // A worker that completed cleanly, with nothing left to answer, starts again from its plain thresholds.
        for worker in workers {
            if worker.state == WorkerState::Completed
                && self
                    .activity
                    .get(&worker.id)
                    .is_some_and(|activity| activity.message.is_none())
            {
                self.episodes.remove(&worker.id);
            }
        }
        let next: HashMap<String, Condition> = workers
            .iter()
            .filter_map(|worker| {
                let activity = self.activity.get(&worker.id)?;
                let episodes = self.episodes.get(&worker.id).copied().unwrap_or(0);
                let backoff = 1 << episodes.min(BACKOFF);
                judge(worker, activity, now, backoff)
                    .map(|condition| (worker.id.clone(), condition))
            })
            .collect();
        // A worker whose activity is unknown keeps what was told of it, until it is known again.
        let activity = &self.activity;
        self.told.retain(|id, kind| {
            !activity.contains_key(id)
                || next
                    .get(id)
                    .is_some_and(|condition| condition.kind == *kind)
        });
        self.pending.retain(|id| next.contains_key(id));
        for worker in workers {
            if let Some(condition) = next.get(&worker.id)
                && self.told.get(&worker.id) != Some(&condition.kind)
                && !self.pending.contains(&worker.id)
            {
                self.pending.push(worker.id.clone());
            }
        }
        let changed = next.len() != self.conditions.len()
            || next.iter().any(|(id, condition)| {
                self.conditions
                    .get(id)
                    .is_none_or(|before| before.kind != condition.kind)
            });
        self.conditions = next;
        changed
    }

    /// Judge `workers` again at the time last given, as when what they are doing changed.
    pub fn judge(&mut self, workers: &[Worker]) -> bool {
        self.tick(self.now, None, workers)
    }

    /// The clock jumped by `shift` since it read `before`: every time held moves by as much. A jump forward, as across a
    /// sleep, is remembered, so that times reported later from no later than `before` move too; after a jump back, the
    /// times reported later cannot be told apart by when they were observed, and are taken as they are.
    fn jump(&mut self, before: u64, shift: i64) {
        for activity in self.activity.values_mut() {
            rebase(activity, (u64::MAX, shift));
        }
        if shift > 0 {
            if self.jumps.len() == JUMPS {
                self.jumps.pop_front();
            }
            self.jumps.push_back((before.saturating_add(1), shift));
        }
        self.last = self.last.map(|last| moved(last, shift));
        for (at, _) in &mut self.delivered {
            *at = moved(*at, shift);
        }
    }

    /// What is wrong with the worker `id`, if anything is.
    pub fn condition(&self, id: &str) -> Option<&Condition> {
        self.conditions.get(id)
    }

    /// `worker` as everyone in the space sees it: stalled or unanswered while it is. A failed worker already shows it.
    pub fn shown(&self, worker: &Worker) -> Worker {
        let mut shown = worker.clone();
        match self
            .conditions
            .get(&worker.id)
            .map(|condition| condition.kind)
        {
            Some(Kind::Stalled) => shown.state = WorkerState::Stalled,
            Some(Kind::Unanswered) => shown.state = WorkerState::Unanswered,
            Some(Kind::Failed) | None => {}
        }
        shown
    }

    /// Whether notes about workers wait to be told, and may be: a failure at once, the rest by the pace.
    pub fn due(&self) -> bool {
        self.paced() && !self.pending.is_empty()
            || self.pending.iter().any(|id| {
                self.conditions
                    .get(id)
                    .is_some_and(|condition| condition.kind == Kind::Failed)
            })
    }

    /// Whether notes about stalled or unanswered workers may be told now, by the pace.
    fn paced(&self) -> bool {
        self.last.is_none_or(|last| self.now >= last + PACE)
    }

    /// The notes to tell the agent now, about the workers of `backend` in `workers`, one for each condition, without
    /// their brackets, keyed by the condition, if any are due: each worker is told once until it recovers. See
    /// [`Self::due`].
    ///
    /// At most [`DAILY`] deliveries are made in a day, and [`UNATTENDED`] of them while nobody is `present`; beyond
    /// those, what falls due is counted as told, and logged, rather than told.
    pub fn take(
        &mut self,
        backend: &str,
        workers: &[Worker],
        present: bool,
    ) -> Vec<(&'static str, String)> {
        if !self.due() {
            return Vec::new();
        }
        let now = self.now;
        while self
            .delivered
            .front()
            .is_some_and(|(at, _)| now.saturating_sub(*at) >= DAY)
        {
            self.delivered.pop_front();
        }
        let unattended = self
            .delivered
            .iter()
            .filter(|(_, present)| !present)
            .count();
        // At the most for the day, the agent is told once that notes are paused, and until when; what falls due
        // meanwhile waits, untold, to be judged again then.
        let until = if self.delivered.len() >= DAILY {
            self.delivered
                .front()
                .map(|(at, _)| (at + DAY, "the most for a day"))
        } else if !present && unattended >= UNATTENDED {
            self.delivered
                .iter()
                .find(|(_, present)| !present)
                .map(|(at, _)| (at + DAY, "the most while nobody is in the space"))
        } else {
            None
        };
        if let Some((until, why)) = until {
            if self.paused.is_some() {
                return Vec::new();
            }
            self.paused = Some(until);
            log::info!("Notes about workers are paused: the space had {why}.");
            let delivered = if why == "the most for a day" {
                DAILY
            } else {
                UNATTENDED
            };
            return vec![(
                "paused",
                format!(
                    "Notes about workers are paused until {}, having reached {why}, {delivered}; look at the workers tool meanwhile",
                    clock_time(until)
                ),
            )];
        }
        self.paused = None;
        let kinds: &[Kind] = if self.paced() {
            &[Kind::Failed, Kind::Unanswered, Kind::Stalled]
        } else {
            &[Kind::Failed]
        };
        let mut notes = Vec::new();
        for &kind in kinds {
            let members: Vec<(&Worker, &Condition)> = workers
                .iter()
                .filter(|worker| self.pending.contains(&worker.id))
                .filter_map(|worker| {
                    self.conditions
                        .get(&worker.id)
                        .filter(|condition| condition.kind == kind)
                        .map(|condition| (worker, condition))
                })
                .collect();
            if members.is_empty() {
                continue;
            }
            for (worker, _) in &members {
                self.told.insert(worker.id.clone(), kind);
                if kind != Kind::Failed {
                    *self.episodes.entry(worker.id.clone()).or_default() += 1;
                }
            }
            notes.push((kind.name(), self.note(backend, kind, &members)));
        }
        let told = &self.told;
        let conditions = &self.conditions;
        self.pending.retain(|id| {
            conditions
                .get(id)
                .is_some_and(|condition| told.get(id) != Some(&condition.kind))
        });
        if notes.iter().any(|(kind, _)| *kind != Kind::Failed.name()) {
            self.last = Some(now);
        }
        self.delivered.push_back((now, present));
        notes
    }

    /// The note about `members`, all in the condition `kind`.
    fn note(&self, backend: &str, kind: Kind, members: &[(&Worker, &Condition)]) -> String {
        let now = self.now;
        let backend = one_line(backend, limits::NAME_BYTES);
        let error = |condition: &Condition| {
            condition
                .error
                .as_deref()
                .map(|error| one_line(error, ERROR_CHARS))
        };
        let stalled = |condition: &Condition| match (&condition.tool, &condition.finish) {
            (Some(tool), _) => format!(
                "running {} for {}",
                one_line(&tool.name, TITLE_CHARS),
                minutes(now.saturating_sub(tool.since))
            ),
            (None, Some(finish)) => format!(
                "no progress for {}, its last step ended {}",
                minutes(now.saturating_sub(condition.since)),
                ended(finish)
            ),
            (None, None) => format!(
                "no progress for {}",
                minutes(now.saturating_sub(condition.since))
            ),
        };
        // A message still waiting on a stalled worker, queued behind its run or not yet long unanswered.
        let waiting = |condition: &Condition| {
            condition.sent.map(|sent| {
                format!(
                    "a message sent {} ago is still waiting",
                    minutes(now.saturating_sub(sent))
                )
            })
        };
        let name = |worker: &Worker, evidence: Option<String>| {
            let title = one_line(&worker.title, TITLE_CHARS);
            let session = one_line(&worker.id, limits::SESSION_BYTES);
            match evidence {
                Some(evidence) => format!("\"{title}\" ({backend} session {session}, {evidence})"),
                None => format!("\"{title}\" ({backend} session {session})"),
            }
        };
        if let [(worker, condition)] = members {
            let named = name(worker, None);
            return match kind {
                Kind::Failed => match error(condition) {
                    Some(error) => format!("Worker {named} failed: {error}"),
                    None => format!("Worker {named} failed"),
                },
                Kind::Unanswered => format!(
                    "Worker {named} has not answered a message sent {} ago",
                    minutes(now.saturating_sub(condition.sent.unwrap_or(condition.since)))
                ),
                Kind::Stalled => {
                    let note = match &condition.tool {
                        Some(_) => format!(
                            "Worker {named} has been {} with no other progress",
                            stalled(condition)
                        ),
                        None => format!("Worker {named} has shown {}", stalled(condition)),
                    };
                    match waiting(condition) {
                        Some(waiting) => format!("{note}; {waiting}"),
                        None => note,
                    }
                }
            };
        }
        match kind {
            Kind::Failed => {
                let first = error(members[0].1);
                let same = members
                    .iter()
                    .all(|(_, condition)| error(condition) == first);
                let names: Vec<String> = members
                    .iter()
                    .map(|(worker, condition)| {
                        name(worker, if same { None } else { error(condition) })
                    })
                    .collect();
                match first.filter(|_| same) {
                    Some(error) => format!("Workers {} failed: {error}", listed(&names)),
                    None => format!("Workers {} failed", listed(&names)),
                }
            }
            Kind::Unanswered => {
                let names: Vec<String> = members
                    .iter()
                    .map(|(worker, condition)| {
                        let sent = format!(
                            "sent {} ago",
                            minutes(now.saturating_sub(condition.sent.unwrap_or(condition.since)))
                        );
                        name(worker, Some(sent))
                    })
                    .collect();
                format!("Workers {} have not answered a message", listed(&names))
            }
            Kind::Stalled => {
                let names: Vec<String> = members
                    .iter()
                    .map(|(worker, condition)| {
                        let evidence = match waiting(condition) {
                            Some(waiting) => format!("{}, {waiting}", stalled(condition)),
                            None => stalled(condition),
                        };
                        name(worker, Some(evidence))
                    })
                    .collect();
                format!("Workers {} have shown no progress", listed(&names))
            }
        }
    }
}

#[cfg(test)]
mod tests;
