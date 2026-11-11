// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The agent's timers: `set_timer`, `timers` and `cancel_timer`, each acting on the space bound to the calling session,
//! and the hub looking at every space's timers as the clock moves, telling each agent what fired through its space's
//! event queue. See `design/timers.md` and `design/mcp.md`.
use serde_json::{Value, json};
use std::time::Instant;

use super::{Hub, HubState};
use crate::timers::{Request, Timer, schedule::instant};

/// What the agent is told to do once it has set or replaced a timer.
const SET: &str = "Tell whoever asked, in a sentence, what you set and when it next fires. When it fires, Tomori tells you in square brackets, as a note of its own, with your note: decide then what to do, if anything; it is not a request from anyone.";

impl HubState {
    /// The names of the participants present in `space`: owners and collaborators in it whose client reports them
    /// engaged or distracted, wearing or attending to it. A visitor, a client that reports nothing, such as the
    /// spectator, and anyone away or gone are not. See `design/timers.md`.
    pub(super) fn present(&self, space: &str) -> Vec<String> {
        self.space(space).map_or_else(Vec::new, |record| {
            record
                .model
                .participants
                .iter()
                .filter_map(|key| self.participants.get(key))
                .filter(|participant| {
                    participant.connection.is_some()
                        && matches!(
                            participant.engagement,
                            Some(
                                tomori_protocol::Engagement::Engaged
                                    | tomori_protocol::Engagement::Distracted
                            )
                        )
                })
                .map(|participant| participant.name.clone())
                .collect()
        })
    }
}

impl Hub {
    /// The time the agent's timers go by: the clock's, or the time a test set.
    pub(crate) fn wall_now(&self) -> jiff::Timestamp {
        self.clock
            .read()
            .expect("never left poisoned")
            .unwrap_or_else(jiff::Timestamp::now)
    }

    /// Have the agent's timers go by `now` rather than the clock, as a test does.
    #[cfg(test)]
    pub(crate) fn set_clock(&self, now: jiff::Timestamp) {
        *self.clock.write().expect("never left poisoned") = Some(now);
    }

    /// Have timers set without a zone use `zone`, rather than this machine's, as a test does.
    #[cfg(test)]
    pub(crate) fn set_zone(&self, zone: &str) {
        *self.zone.write().expect("never left poisoned") = zone.to_owned();
    }

    /// Look at the timers of every space whose agent can be told at `now`: open, bound to its session and connected to
    /// it. What fires waits in the space's event queue for the agent's turn to end, and is handed over when it may be;
    /// a space whose timers look different is sent to everyone in it, and what is kept is written. A closed space, or one
    /// not yet bound again after a restart, is left as it is, so that what fell due meanwhile is caught up once it is.
    /// The space's workers are judged at `now` too, as `design/events.md` describes. See `design/timers.md`.
    pub fn tick_timers(&self, now: jiff::Timestamp) {
        let mut changed = false;
        // A sleep or a jump of the clock is noticed against the monotonic clock, unless a test gives the time.
        let monotonic = self
            .clock
            .read()
            .expect("never left poisoned")
            .is_none()
            .then(Instant::now);
        {
            let mut state = self.lock();
            let ids: Vec<String> = state
                .spaces
                .iter()
                .filter(|record| {
                    !record.restoring
                        && !record.model.is_closed()
                        && !record.model.is_reconnecting()
                        && (!record.model.timers().list().is_empty()
                            || record.model.has_events()
                            || record.model.watching_workers())
                })
                .map(|record| record.model.id.clone())
                .collect();
            for id in ids {
                let present = state.present(&id);
                let record = state.space_mut(&id).expect("listed above");
                let timers = record.model.tick_timers(now, &present);
                // Its workers go by the same clock, shown changed for everyone but not kept. See `design/events.md`.
                let workers = record
                    .model
                    .tick_workers(u64::try_from(now.as_millisecond()).unwrap_or(0), monotonic);
                if timers || workers {
                    state.touch_space(&id);
                }
                changed |= timers;
                state.pump(&id);
            }
        }
        if changed {
            self.keep();
        }
    }

    /// The soonest occurrence after `now` of a timer in a space whose timers [`Self::tick_timers`] looks at, so that the
    /// server looks at them then, within milliseconds of it, rather than at its next sweep. Occurrences already due are
    /// left to the sweep, so that one held back, as after a restart, never has the server look again at once.
    pub fn next_timer_due(&self, now: jiff::Timestamp) -> Option<jiff::Timestamp> {
        let now = now.as_second();
        let state = self.lock();
        state
            .spaces
            .iter()
            .filter(|record| {
                !record.restoring && !record.model.is_closed() && !record.model.is_reconnecting()
            })
            .filter_map(|record| record.model.timers().next_due())
            .filter(|due| *due > now)
            .min()
            .map(instant)
    }

    /// Set the timer `request` asks for, in the space bound to `session`, replacing one of the same name, as its agent
    /// asked: everyone in it sees it, and it is kept and logged. See `design/timers.md`.
    pub(crate) fn agent_set_timer(&self, session: &str, request: Request) -> Result<Value, String> {
        let now = self.wall_now();
        let zone = self.zone.read().expect("never left poisoned").clone();
        let (space, timer, replaced) = {
            let mut state = self.lock();
            let space = self.calling_space(&state, session)?;
            let record = state.space_mut(&space).expect("found above");
            let (timer, replaced) = record.model.timers_mut().set(request, now, &zone)?;
            state.touch_space(&space);
            (space, timer, replaced)
        };
        self.keep();
        log::info!(
            "The agent of space {space} {} timer \"{}\" ({}).",
            if replaced { "replaced" } else { "set" },
            timer.name,
            timer.describe()
        );
        Ok(json!({
            "space": space,
            "timer": described(&timer, now),
            "replaced": replaced,
            "next": SET,
        }))
    }

    /// The timers of the space bound to `session`, with the limits on them.
    pub(crate) fn tool_timers(&self, session: &str) -> Result<Value, String> {
        let now = self.wall_now();
        let state = self.lock();
        let space = self.calling_space(&state, session)?;
        let record = state.space(&space).expect("found above");
        let timers: Vec<Value> = record
            .model
            .timers()
            .list()
            .iter()
            .map(|timer| described(timer, now))
            .collect();
        Ok(json!({
            "space": space,
            "timers": timers,
            "most": crate::timers::MOST,
            "daily_firings": crate::timers::DAILY_FIRINGS,
            "unattended_firings": crate::timers::UNATTENDED_FIRINGS,
        }))
    }

    /// Cancel the timer called `name` in the space bound to `session`, as its agent asked: everyone in it sees it go,
    /// and it is kept and logged.
    pub(crate) fn agent_cancel_timer(&self, session: &str, name: &str) -> Result<Value, String> {
        let (space, timer) = {
            let mut state = self.lock();
            let space = self.calling_space(&state, session)?;
            let record = state.space_mut(&space).expect("found above");
            let timers = record.model.timers_mut();
            let Some(timer) = timers.cancel(name) else {
                let names: Vec<&str> = timers
                    .list()
                    .iter()
                    .map(|timer| timer.name.as_str())
                    .collect();
                return Err(if names.is_empty() {
                    "This space has no timers.".to_owned()
                } else {
                    format!(
                        "This space has no timer called {:?}; its timers are {}.",
                        crate::space::one_line(name, crate::timers::NAME_CHARS),
                        names.join(", ")
                    )
                });
            };
            state.touch_space(&space);
            (space, timer)
        };
        self.keep();
        log::info!(
            "The agent of space {space} cancelled timer \"{}\".",
            timer.name
        );
        Ok(json!({
            "space": space,
            "cancelled": timer.name,
            "next": "Tell whoever asked that it is cancelled.",
        }))
    }
}

/// A timer as the agent's tools describe it at `now`: its name, note, schedule and condition in short words, zone,
/// when it next fires, in its zone, to the second for a timer set in seconds, and how long from now, and what it has
/// done.
fn described(timer: &Timer, now: jiff::Timestamp) -> Value {
    let known = crate::timers::zone(&timer.zone);
    let unknown = known.is_err();
    let zone = known.unwrap_or(jiff::tz::TimeZone::UTC);
    let format = if timer.schedule.to_the_second(&zone) {
        "%a %Y-%m-%d %H:%M:%S"
    } else {
        "%a %Y-%m-%d %H:%M"
    };
    let local = |at: i64| zone.to_datetime(instant(at)).strftime(format).to_string();
    json!({
        "name": timer.name,
        "note": timer.note,
        "schedule": timer.schedule.describe(&zone),
        "condition": timer.condition,
        "zone": timer.zone,
        "zone_note": unknown.then_some("Tomori does not know this time zone, so this timer goes by UTC: set it again with a zone Tomori knows."),
        "next": timer.next.map(local),
        "next_in": timer.next.map(|next| crate::timers::until(instant(next), now)),
        "waiting_for_arrival": timer.waiting.as_ref().map(|waiting| local(waiting.due)),
        "until": timer.until.map(local),
        "count": timer.count,
        "fired": timer.fired,
        "last_fired": timer.last_fired.map(local),
    })
}
