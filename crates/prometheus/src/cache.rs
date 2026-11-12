// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! A client's cache: identical requests in flight together share one, answers are kept while they are fresh, a range query's points are kept so that the next read of its window asks only for the steps it lacks, partial answers are never kept, and what is kept is bounded, the least recently used dropped first.
use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap, HashMap};
use std::mem::size_of;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use crate::{Data, Error, Labels, Point, Range, Response, Sample, Series};

/// How many answers a cache keeps unless the application sets another bound.
pub const DEFAULT_CACHE_ENTRIES: usize = 1024;

/// How many bytes of answers a cache keeps, by its own estimate, unless the application sets another bound: 64 MiB.
pub const DEFAULT_CACHE_BYTES: usize = 64 * 1024 * 1024;

/// How much of a range query's newest points kept are asked for again with the steps it lacks, unless the application sets another overlap: a minute, for samples that reach the source late.
pub const DEFAULT_OVERLAP: Duration = Duration::from_secs(60);

/// How long a range query's points kept reach back as far as the longest range asked of them, unless the application sets another decay: once no range that long has been asked for over this time and the one before it, they reach back only as far as the longest asked since.
pub const DEFAULT_SPAN_DECAY: Duration = Duration::from_secs(15 * 60);

/// The configuration of a client's cache, for [`crate::Builder::cache`].
///
/// A client with a cache sends one request for identical calls in flight together, the others waiting for its answer (see [`crate::Builder::cache`] for which wait); keeps an answer for as long as it is fresh, [`Cache::fresh`] or the call's own ([`crate::Call::fresh`], [`crate::Call::bucket`]), so that another call for it within that time is answered without a request; and keeps a range query's points, so that a later read of the same expression at the same step, its window moved on, asks only for the steps it lacks, with [`Cache::overlap`] of those it has again. An answer that may be partial ([`Response::partial`]) is returned but never kept, and what was kept of its range is dropped, so that the next read asks for its whole window again; an error is never kept, nor shared with calls that come after it. A range query whose expression evaluates at its range's start or end (`@ start()`, `@ end()`) is read whole every time and never kept, since its every point depends on the range, as query frontends treat it.
///
/// ```
/// use std::time::Duration;
/// let cache = tomori_prometheus::Cache::new()
///     .fresh(Duration::from_secs(15))
///     .max_entries(256)
///     .max_bytes(16 * 1024 * 1024);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cache {
    max_entries: usize,
    max_bytes: usize,
    fresh: Duration,
    overlap: Duration,
    span_decay: Duration,
}

impl Default for Cache {
    fn default() -> Self {
        Self::new()
    }
}

impl Cache {
    /// A cache of at most [`DEFAULT_CACHE_ENTRIES`] answers and [`DEFAULT_CACHE_BYTES`], keeping an instant query's answer only for calls that say how long it is fresh, and a range query's points with an overlap of [`DEFAULT_OVERLAP`] and a decay of [`DEFAULT_SPAN_DECAY`].
    pub fn new() -> Self {
        Self {
            max_entries: DEFAULT_CACHE_ENTRIES,
            max_bytes: DEFAULT_CACHE_BYTES,
            fresh: Duration::ZERO,
            overlap: DEFAULT_OVERLAP,
            span_decay: DEFAULT_SPAN_DECAY,
        }
    }

    /// The most answers kept; the least recently used is dropped to keep a new one beyond it.
    pub fn max_entries(mut self, entries: usize) -> Self {
        self.max_entries = entries;
        self
    }

    /// The most bytes of answers kept, by the cache's estimate of the memory their labels, points, text and the cache's own bookkeeping take, allocated capacity included; the least recently used are dropped to keep a new one beyond it, and an answer larger than this is never kept.
    pub fn max_bytes(mut self, bytes: usize) -> Self {
        self.max_bytes = bytes;
        self
    }

    /// How long an answer is fresh unless a call says otherwise: another call for it within this time is answered from the cache. Make it no longer than the data's resolution, such as half the source's scrape interval; zero, the default, keeps no instant query's answer and answers no range query wholly from the cache, though calls in flight together still share a request and range queries still ask only for the steps they lack.
    pub fn fresh(mut self, fresh: Duration) -> Self {
        self.fresh = fresh;
        self
    }

    /// How much of a range query's newest points kept are asked for again with the steps it lacks, rounded up to whole steps, for samples that reached the source after the points were read; [`DEFAULT_OVERLAP`] otherwise.
    pub fn overlap(mut self, overlap: Duration) -> Self {
        self.overlap = overlap;
        self
    }

    /// How long a range query's points kept go on reaching back as far as a range no longer asked for; [`DEFAULT_SPAN_DECAY`] otherwise. They reach back as far as the longest range asked of them in the current period of this length and the one before, so that a longer range asked now and then keeps its points, while one no longer asked lets them shrink.
    pub fn span_decay(mut self, decay: Duration) -> Self {
        self.span_decay = decay;
        self
    }
}

/// What a client's cache has done since the client was built, and what it holds, from [`crate::Client::cache_stats`]. A client without a cache has none of either.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CacheStats {
    /// Calls answered from the cache without a request.
    pub hits: u64,
    /// Range queries answered from the cache with a request for only the steps it lacked.
    pub tails: u64,
    /// Calls that sent a request for all they asked.
    pub misses: u64,
    /// Calls that waited for an identical call's request in flight rather than send their own.
    pub deduped: u64,
    /// Answers that may be partial, which were returned and not kept.
    pub partial: u64,
    /// Answers dropped to keep the cache within its bounds.
    pub evictions: u64,
    /// Answers kept now.
    pub entries: usize,
    /// Bytes kept now, by the cache's estimate.
    pub bytes: usize,
}

/// What an answer is kept under: an instant query's expression and time, or a range query's expression, step and phase, the milliseconds its points lie past a multiple of the step, so that ranges whose points coincide share their points.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Key {
    Instant {
        expression: String,
        at: i64,
    },
    Range {
        expression: String,
        step: i64,
        phase: i64,
    },
}

impl Key {
    pub fn range(expression: &str, range: &Range) -> Self {
        let step = step_millis(range);
        Self::Range {
            expression: expression.to_owned(),
            step,
            phase: range.start.millis().rem_euclid(step),
        }
    }

    fn expression(&self) -> &str {
        match self {
            Self::Instant { expression, .. } | Self::Range { expression, .. } => expression,
        }
    }
}

/// A request in flight, which identical calls wait for: its key, and for a range query the window it was asked for.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Asked {
    key: Key,
    start: i64,
    end: i64,
}

/// How a call's request is sent, which decides whose request it may wait for: its priority and its time-out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Sending {
    pub background: bool,
    pub timeout: Duration,
}

impl Sending {
    /// Whether a call sent as this may wait for a request sent as `leader`: one of the same priority, so that a call never waits behind a background request's turn for a slot nor makes one wait as its own, with a time-out at least as long, so that it never fails sooner than its own would.
    fn joins(&self, leader: &Sending) -> bool {
        self.background == leader.background && leader.timeout >= self.timeout
    }
}

type Outcome = Result<Response<Data>, Error>;

/// A request in flight's outcome, once it has one, for the calls waiting for it.
pub(crate) struct Flight {
    sending: Sending,
    outcome: Mutex<Option<Outcome>>,
    landed: Condvar,
}

impl Flight {
    /// The outcome of the request waited for, or a time-out of `timeout` once twice that has passed without one, as long as a call's own request could wait for a slot and then for its answer.
    pub fn wait(&self, timeout: Duration) -> Outcome {
        let deadline = Instant::now() + timeout.saturating_mul(2);
        let mut outcome = self.outcome.lock().unwrap_or_else(PoisonError::into_inner);
        loop {
            if let Some(outcome) = &*outcome {
                return outcome.clone();
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(Error::Timeout(timeout));
            }
            outcome = self
                .landed
                .wait_timeout(outcome, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }
}

/// The request a call leads, which identical calls wait for until it lands, even if the call ends without landing it, as a panic would.
pub(crate) struct Landing<'a> {
    store: &'a Store,
    asked: Asked,
    flight: Arc<Flight>,
    landed: bool,
}

impl Landing<'_> {
    fn land(mut self, outcome: &Outcome) {
        self.finish(outcome.clone());
    }

    fn finish(&mut self, outcome: Outcome) {
        self.landed = true;
        let mut state = self.store.state();
        let flights = state.flights.entry(self.asked.clone()).or_default();
        flights.retain(|flight| !Arc::ptr_eq(flight, &self.flight));
        if flights.is_empty() {
            state.flights.remove(&self.asked);
        }
        drop(state);
        *self
            .flight
            .outcome
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(outcome);
        self.flight.landed.notify_all();
    }
}

impl Drop for Landing<'_> {
    fn drop(&mut self) {
        if !self.landed {
            self.finish(Err(Error::Transport(
                "the identical request this one waited for failed".into(),
            )));
        }
    }
}

/// How a call begins: answered from the cache, waiting for an identical request in flight, or sending its own.
pub(crate) enum Begin<'a, T, P> {
    Hit(Response<T>),
    Follow(Arc<Flight>),
    Lead(Landing<'a>, P),
}

/// What a range query asks for: from `from`, its window's start unless the cache has the points before it, which are `head`; and whether its answer may be kept.
pub(crate) struct Plan {
    pub from: i64,
    head: Vec<Series>,
    keep: bool,
}

/// A client's cache.
pub(crate) struct Store {
    config: Cache,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    entries: HashMap<Key, Entry>,
    flights: HashMap<Asked, Vec<Arc<Flight>>>,
    /// Each entry by when it was last used, the least recent first.
    used: BTreeMap<u64, Key>,
    /// When each instant query's answer stops being fresh, the soonest first, with the entry's use then, so that one kept again since is told apart.
    expiries: BinaryHeap<Reverse<(Instant, u64, Key)>>,
    /// Counts each use.
    clock: u64,
    bytes: usize,
    stats: CacheStats,
}

struct Entry {
    kept: Kept,
    /// When its newest points were read, `None` for points remembered rather than read, which are never fresh.
    fetched: Option<Instant>,
    /// When it was kept, which its place in `expiries` names.
    kept_at: u64,
    used: u64,
    bytes: usize,
}

enum Kept {
    Instant(Response<Data>),
    Range(Window),
}

/// A range query's points kept: those from `start` to `end`, both points of its step, over at most the longest range asked for recently ([`Spans`]).
struct Window {
    start: i64,
    end: i64,
    spans: Spans,
    series: Vec<Series>,
    warnings: Vec<String>,
}

/// The longest ranges asked of a window kept, in this period of the cache's decay and the one before.
#[derive(Clone, Copy)]
struct Spans {
    now: i64,
    before: i64,
    since: Instant,
}

impl Spans {
    fn new(span: i64) -> Self {
        Self {
            now: span,
            before: 0,
            since: Instant::now(),
        }
    }

    /// Note a range of `span` asked for, starting a new period once `decay` has passed.
    fn ask(&mut self, span: i64, decay: Duration) {
        if self.since.elapsed() >= decay {
            self.before = self.now;
            self.now = span;
            self.since = Instant::now();
        } else {
            self.now = self.now.max(span);
        }
    }

    fn longest(&self) -> i64 {
        self.now.max(self.before)
    }
}

impl Store {
    pub fn new(config: Cache) -> Self {
        Self {
            config,
            state: Mutex::new(State::default()),
        }
    }

    pub fn config(&self) -> &Cache {
        &self.config
    }

    /// How long an answer is fresh unless a call says otherwise.
    pub fn config_fresh(&self) -> Duration {
        self.config.fresh
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn stats(&self) -> CacheStats {
        let state = self.state();
        CacheStats {
            entries: state.entries.len(),
            bytes: state.bytes,
            ..state.stats
        }
    }

    /// Lead the request for `asked`, sent as `sending`, or follow an identical one in flight that it may wait for.
    fn fly<'a, T, P>(
        &'a self,
        state: &mut State,
        asked: Asked,
        sending: Sending,
        plan: P,
    ) -> Begin<'a, T, P> {
        let flights = state.flights.entry(asked.clone()).or_default();
        if let Some(flight) = flights.iter().find(|flight| sending.joins(&flight.sending)) {
            let flight = flight.clone();
            state.stats.deduped += 1;
            return Begin::Follow(flight);
        }
        let flight = Arc::new(Flight {
            sending,
            outcome: Mutex::new(None),
            landed: Condvar::new(),
        });
        flights.push(flight.clone());
        Begin::Lead(
            Landing {
                store: self,
                asked,
                flight,
                landed: false,
            },
            plan,
        )
    }

    /// Begin an instant query kept under `key`, whose answer is fresh for `fresh`, sent as `sending`.
    pub fn begin_instant(
        &self,
        key: Key,
        fresh: Duration,
        sending: Sending,
    ) -> Begin<'_, Data, ()> {
        let mut state = self.state();
        if let Some(entry) = state.entries.get(&key)
            && let Kept::Instant(response) = &entry.kept
            && entry
                .fetched
                .is_some_and(|fetched| fetched.elapsed() < fresh)
        {
            let response = response.clone();
            state.touch(&key);
            state.stats.hits += 1;
            return Begin::Hit(response);
        }
        let asked = Asked {
            key,
            start: 0,
            end: 0,
        };
        let begun = self.fly(&mut state, asked, sending, ());
        if matches!(begun, Begin::Lead(..)) {
            state.stats.misses += 1;
        }
        begun
    }

    /// Keep an instant query's `outcome`, fresh for `fresh`, unless it failed or may be partial, and give it to the calls that waited for it.
    pub fn finish_instant(&self, landing: Landing<'_>, fresh: Duration, outcome: &Outcome) {
        if let Ok(response) = outcome {
            let mut state = self.state();
            if response.partial {
                state.stats.partial += 1;
                state.remove(&landing.asked.key);
            } else if !fresh.is_zero() {
                let bytes = weigh_data(&response.data) + weigh_warnings(&response.warnings);
                state.keep(
                    &self.config,
                    landing.asked.key.clone(),
                    Kept::Instant(response.clone()),
                    Some(Instant::now()),
                    Some(fresh),
                    bytes,
                );
            }
        }
        landing.land(outcome);
    }

    /// Begin a range query of `expression` over `range`, whose step is at least a millisecond, kept under `key`, sent as `sending`, wholly answered from what is kept if it has every step of the window and is fresh for `fresh`. An expression evaluated at the range's start or end is read whole and never kept.
    pub fn begin_range(
        &self,
        key: Key,
        range: &Range,
        fresh: Duration,
        sending: Sending,
    ) -> Begin<'_, Vec<Series>, Plan> {
        let step = step_millis(range);
        let start = range.start.millis();
        let last = last_step(range);
        let keep = !crate::promql::anchored(key.expression());
        let mut state = self.state();
        let overlap = whole_steps(self.config.overlap, step);
        let mut plan = Plan {
            from: start,
            head: Vec::new(),
            keep,
        };
        if keep
            && let Some(entry) = state.entries.get(&key)
            && let Kept::Range(window) = &entry.kept
            && start >= window.start
            && start <= window.end.saturating_add(step)
        {
            if last <= window.end
                && entry
                    .fetched
                    .is_some_and(|fetched| fetched.elapsed() < fresh)
            {
                let response = Response {
                    data: slice(&window.series, start, last),
                    warnings: window.warnings.clone(),
                    partial: false,
                };
                state.touch(&key);
                state.stats.hits += 1;
                return Begin::Hit(response);
            }
            let from = window
                .end
                .saturating_add(step)
                .min(last)
                .saturating_sub(overlap)
                .max(start);
            if from > start {
                plan.head = slice(&window.series, start, from - 1);
                plan.from = from;
                state.touch(&key);
            }
        }
        let asked = Asked {
            key,
            start,
            end: range.end.millis(),
        };
        let tail = plan.from > start;
        let begun = self.fly(&mut state, asked, sending, plan);
        if matches!(begun, Begin::Lead(..)) {
            if tail {
                state.stats.tails += 1;
            } else {
                state.stats.misses += 1;
            }
        }
        begun
    }

    /// The range query over `range` from what was kept before `plan.from` and what the request from it answered, `fetched`: the window's points kept, with the newly fetched in place of any kept from `plan.from` on, unless the answer may be partial, when what was kept of it is dropped so that the next read asks for its whole window; and given to the calls that waited for it.
    pub fn finish_range(
        &self,
        landing: Landing<'_>,
        range: &Range,
        plan: Plan,
        fetched: Result<Response<Vec<Series>>, Error>,
    ) -> Result<Response<Vec<Series>>, Error> {
        let outcome = fetched.map(|response| {
            let mut state = self.state();
            if response.partial {
                state.stats.partial += 1;
                state.remove(&landing.asked.key);
            } else if plan.keep {
                state.keep_range(
                    &self.config,
                    &landing.asked.key,
                    range,
                    plan.from,
                    &response,
                );
            }
            Response {
                data: compose(plan.head, response.data),
                warnings: response.warnings,
                partial: response.partial,
            }
        });
        let shared = outcome.clone().map(|response| Response {
            data: Data::Matrix(response.data),
            warnings: response.warnings,
            partial: response.partial,
        });
        landing.land(&shared);
        outcome
    }

    /// Keep `series`, a range query's answer over `range` under `key` that the application kept itself, as though read but never fresh: its points outside `range` left out, and any the cache has read itself kept in place of them. Refused, keeping nothing, if a series' points are not in order or not on the range's steps.
    pub fn remember(&self, key: Key, range: &Range, series: &[Series]) -> Result<(), Error> {
        let step = step_millis(range);
        let (start, last) = (range.start.millis(), last_step(range));
        for one in series {
            let ordered = one.points.windows(2).all(|pair| pair[0].at < pair[1].at);
            let aligned = one
                .points
                .iter()
                .all(|point| (point.at.millis() - start).rem_euclid(step) == 0);
            if !ordered || !aligned {
                return Err(Error::Invalid(
                    "remembered points must be in order, on the range's steps".into(),
                ));
            }
        }
        if crate::promql::anchored(key.expression()) {
            return Ok(());
        }
        let remembered = slice(series, start, last);
        let mut state = self.state();
        let existing = state.remove(&key);
        let (window, fetched) = match existing {
            Some(Entry {
                kept: Kept::Range(mut window),
                fetched,
                ..
            }) if start <= window.end.saturating_add(step)
                && last >= window.start.saturating_sub(step) =>
            {
                // What the cache read itself is newer: the remembered points only add the steps before and after it.
                let before = slice(&remembered, start, window.start - 1);
                let after = slice(&remembered, window.end + 1, last);
                let newer = if after.is_empty() { fetched } else { None };
                window.series = compose(compose(before, window.series), after);
                window.start = window.start.min(start);
                window.end = window.end.max(last);
                window.spans.now = window.spans.now.max(last - start);
                (window, newer)
            }
            Some(entry) => {
                state.reinsert(&self.config, key, entry);
                return Ok(());
            }
            None => (
                Window {
                    start,
                    end: last,
                    spans: Spans::new(last - start),
                    series: remembered,
                    warnings: Vec::new(),
                },
                None,
            ),
        };
        state.keep_window(&self.config, key, window, fetched);
        Ok(())
    }
}

impl State {
    fn remove(&mut self, key: &Key) -> Option<Entry> {
        let entry = self.entries.remove(key)?;
        self.used.remove(&entry.used);
        self.bytes -= entry.bytes;
        Some(entry)
    }

    /// Mark the entry under `key` as used now.
    fn touch(&mut self, key: &Key) {
        self.clock += 1;
        let clock = self.clock;
        let entry = self
            .entries
            .get_mut(key)
            .expect("only a kept entry is used");
        self.used.remove(&entry.used);
        entry.used = clock;
        self.used.insert(clock, key.clone());
    }

    /// Put back a range query's entry taken out but not changed.
    fn reinsert(&mut self, config: &Cache, key: Key, entry: Entry) {
        let bytes = entry.bytes;
        self.insert(config, key, entry.kept, entry.fetched, None, bytes);
    }

    /// Keep `kept` under `key`, of `bytes` besides the bookkeeping, read at `fetched`, an instant query's answer fresh for `fresh`, unless it alone is larger than the cache, dropping instant queries' answers no longer fresh and then the least recently used until the cache is within its bounds.
    fn keep(
        &mut self,
        config: &Cache,
        key: Key,
        kept: Kept,
        fetched: Option<Instant>,
        fresh: Option<Duration>,
        bytes: usize,
    ) {
        self.remove(&key);
        let bytes = bytes.saturating_add(weigh_key(&key));
        self.insert(config, key, kept, fetched, fresh, bytes);
    }

    fn insert(
        &mut self,
        config: &Cache,
        key: Key,
        kept: Kept,
        fetched: Option<Instant>,
        fresh: Option<Duration>,
        bytes: usize,
    ) {
        if bytes > config.max_bytes || config.max_entries == 0 {
            return;
        }
        let now = Instant::now();
        while let Some(Reverse((expiry, kept_at, _))) = self.expiries.peek()
            && *expiry <= now
        {
            let (kept_at, key) = (
                *kept_at,
                self.expiries.pop().map(|Reverse((_, _, key))| key),
            );
            if let Some(key) = key
                && self
                    .entries
                    .get(&key)
                    .is_some_and(|entry| entry.kept_at == kept_at)
            {
                self.remove(&key);
            }
        }
        while self.entries.len() >= config.max_entries
            || self.bytes.saturating_add(bytes) > config.max_bytes
        {
            // Over the bounds with an answer no larger than the cache, so something is kept.
            let (_, oldest) = self.used.pop_first().expect("each entry has a use");
            self.remove(&oldest);
            self.stats.evictions += 1;
        }
        self.clock += 1;
        let clock = self.clock;
        if let (Some(fetched), Some(fresh)) = (fetched, fresh) {
            self.expiries
                .push(Reverse((fetched + fresh, clock, key.clone())));
        }
        self.bytes += bytes;
        self.used.insert(clock, key.clone());
        self.entries.insert(
            key,
            Entry {
                kept,
                fetched,
                kept_at: clock,
                used: clock,
                bytes,
            },
        );
    }

    /// Merge the points `response` answered from `from` to the end of `range` into those kept under `key`, replacing any kept within those steps, then drop those older than the longest range asked for recently; or keep them alone, if they are the whole window and what was kept is not next to them.
    fn keep_range(
        &mut self,
        config: &Cache,
        key: &Key,
        range: &Range,
        from: i64,
        response: &Response<Vec<Series>>,
    ) {
        let step = step_millis(range);
        let start = range.start.millis();
        let last = last_step(range);
        let asked = last - start;
        let adjoins = |entry: &Entry| match &entry.kept {
            Kept::Range(window) => {
                from <= window.end.saturating_add(step) && last >= window.start.saturating_sub(step)
            }
            Kept::Instant(_) => unreachable!("a range query's key keeps its points"),
        };
        let kept = self.entries.get(key).is_some_and(adjoins);
        // What another call has kept meanwhile, apart from these points, is left as it is.
        if from > start && !kept {
            return;
        }
        let read = Some(Instant::now());
        let (window, fetched) = match self.remove(key) {
            Some(Entry {
                kept: Kept::Range(mut window),
                fetched,
                ..
            }) if kept => {
                // The newest points kept are as fresh as when they were last read, which a read ending before them leaves as it was.
                let fetched = if last < window.end { fetched } else { read };
                splice(&mut window.series, from, last, &response.data);
                window.start = window.start.min(from);
                window.end = window.end.max(last);
                window.spans.ask(asked, config.span_decay);
                window.warnings.clone_from(&response.warnings);
                (window, fetched)
            }
            _ => (
                Window {
                    start,
                    end: last,
                    spans: Spans::new(asked),
                    series: response.data.clone(),
                    warnings: response.warnings.clone(),
                },
                read,
            ),
        };
        self.keep_window(config, key.clone(), window, fetched);
    }

    /// Keep `window` under `key`, read at `fetched`, its points older than the longest range asked of it recently dropped and its memory given back.
    fn keep_window(
        &mut self,
        config: &Cache,
        key: Key,
        mut window: Window,
        fetched: Option<Instant>,
    ) {
        let oldest = window.end - window.spans.longest();
        if window.start < oldest {
            window.start = oldest;
            for series in &mut window.series {
                let before = series
                    .points
                    .partition_point(|point| point.at.millis() < oldest);
                series.points.drain(..before);
            }
        }
        window.series.retain(|series| !series.points.is_empty());
        for series in &mut window.series {
            series.points.shrink_to_fit();
        }
        window.series.shrink_to_fit();
        let bytes = weigh_series(&window.series) + weigh_warnings(&window.warnings);
        self.keep(config, key, Kept::Range(window), fetched, None, bytes);
    }
}

fn step_millis(range: &Range) -> i64 {
    i64::try_from(range.step.as_millis())
        .unwrap_or(i64::MAX)
        .max(1)
}

/// The last step of `range`, its last point.
fn last_step(range: &Range) -> i64 {
    let (start, end, step) = (range.start.millis(), range.end.millis(), step_millis(range));
    end - (end - start).rem_euclid(step)
}

/// `duration` in milliseconds, rounded up to whole steps of `step`.
fn whole_steps(duration: Duration, step: i64) -> i64 {
    let millis = i64::try_from(duration.as_millis()).unwrap_or(i64::MAX);
    millis
        .div_euclid(step)
        .saturating_add(i64::from(millis % step != 0))
        * step
}

/// The points of `series` from `start` to `end`, both included, leaving out series with none.
fn slice(series: &[Series], start: i64, end: i64) -> Vec<Series> {
    series
        .iter()
        .filter_map(|series| {
            let points = &series.points;
            let from = points.partition_point(|point| point.at.millis() < start);
            let to = points.partition_point(|point| point.at.millis() <= end);
            (from < to).then(|| Series {
                metric: series.metric.clone(),
                points: points[from..to].to_vec(),
            })
        })
        .collect()
}

/// Which of the series with the same labels each of `series` is, so that series a label filter left alike are kept apart, in the order the source gave them.
fn identities(series: &[Series]) -> Vec<(Labels, usize)> {
    let mut seen: HashMap<&Labels, usize> = HashMap::new();
    series
        .iter()
        .map(|series| {
            let count = seen.entry(&series.metric).or_default();
            *count += 1;
            (series.metric.clone(), *count - 1)
        })
        .collect()
}

/// `head`'s points, all before `tail`'s, followed by `tail`'s, each series of `tail` joined to the one of `head` with the same labels, leaving out series with none.
fn compose(mut head: Vec<Series>, tail: Vec<Series>) -> Vec<Series> {
    let index: HashMap<(Labels, usize), usize> = identities(&head)
        .into_iter()
        .enumerate()
        .map(|(position, identity)| (identity, position))
        .collect();
    let identities = identities(&tail);
    for (series, identity) in tail.into_iter().zip(identities) {
        match index.get(&identity) {
            Some(&position) => head[position].points.extend(series.points),
            None => head.push(series),
        }
    }
    head.retain(|series| !series.points.is_empty());
    head
}

/// Replace `kept`'s points from `from` to `to`, both included, with `fetched`'s, in place: each series of `fetched` in the one of `kept` with the same labels, those of `kept` it lacks left with none in those steps, those new to `kept` added. Series left with no points are kept, so that series alike keep their places, until the window is kept.
fn splice(kept: &mut Vec<Series>, from: i64, to: i64, fetched: &[Series]) {
    let index: HashMap<(Labels, usize), usize> = identities(kept)
        .into_iter()
        .enumerate()
        .map(|(position, identity)| (identity, position))
        .collect();
    let mut matched = vec![false; kept.len()];
    for (series, identity) in fetched.iter().zip(identities(fetched)) {
        match index.get(&identity) {
            Some(&position) => {
                matched[position] = true;
                let points = &mut kept[position].points;
                let start = points.partition_point(|point| point.at.millis() < from);
                let end = points.partition_point(|point| point.at.millis() <= to);
                points.splice(start..end, series.points.iter().copied());
            }
            None => kept.push(series.clone()),
        }
    }
    for (series, matched) in kept.iter_mut().zip(matched) {
        if !matched {
            let points = &mut series.points;
            let start = points.partition_point(|point| point.at.millis() < from);
            let end = points.partition_point(|point| point.at.millis() <= to);
            points.drain(start..end);
        }
    }
}

/// The memory a string takes, its allocation included.
fn weigh_string(text: &str, capacity: usize) -> usize {
    size_of::<String>() + capacity.max(text.len())
}

/// The memory a series' labels take: each name and value, and a share of the map's nodes for each, about three words and a pointer.
fn weigh_labels(labels: &Labels) -> usize {
    size_of::<Labels>()
        + labels
            .iter()
            .map(|(name, value)| {
                weigh_string(name, name.capacity())
                    + weigh_string(value, value.capacity())
                    + 4 * size_of::<usize>()
            })
            .sum::<usize>()
}

/// The memory `series` take: the vector's allocation, and each series' labels and points' allocation.
fn weigh_series(series: &Vec<Series>) -> usize {
    size_of::<Vec<Series>>()
        + series.capacity() * size_of::<Series>()
        + series
            .iter()
            .map(|series| {
                weigh_labels(&series.metric) + series.points.capacity() * size_of::<Point>()
            })
            .sum::<usize>()
}

fn weigh_warnings(warnings: &Vec<String>) -> usize {
    size_of::<Vec<String>>()
        + warnings.capacity() * size_of::<String>()
        + warnings
            .iter()
            .map(|warning| warning.capacity())
            .sum::<usize>()
}

fn weigh_data(data: &Data) -> usize {
    size_of::<Data>()
        + match data {
            Data::Vector(samples) => {
                samples.capacity() * size_of::<Sample>()
                    + samples
                        .iter()
                        .map(|sample| weigh_labels(&sample.metric))
                        .sum::<usize>()
            }
            Data::Matrix(series) => weigh_series(series),
            Data::Scalar(_) => 0,
            Data::String(_, text) => text.capacity(),
        }
}

/// The memory the cache's bookkeeping takes for an entry under `key`: the key, twice more in the index of uses and of expiries, the entry, and the hash map's share.
fn weigh_key(key: &Key) -> usize {
    3 * (size_of::<Key>() + key.expression().len())
        + size_of::<Entry>()
        + size_of::<Reverse<(Instant, u64, Key)>>()
        + 2 * size_of::<u64>()
        + 4 * size_of::<usize>()
}
