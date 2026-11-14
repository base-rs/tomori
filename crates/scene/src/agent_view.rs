// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The agent's state as the server says it, smoothed for showing: a more pressing state shows at once, and a less
//! pressing one only once it has lasted [`BRIEF`], or once the state shown has been superseded for [`STALE`], so that
//! the status pill, the swarm and the cues never flicker through a state that lasts a moment, nor keep an old one on
//! screen through a run of brief ones. Each space's state is its own: a sequence number means nothing in another space.
//! See `design/agent-state.md`.
use std::time::{Duration, Instant};

use tomori_protocol::{AgentState, AgentStatus};

/// How long a less pressing state must last before it shows in place of the one shown.
pub const BRIEF: Duration = Duration::from_millis(400);

/// The longest a state shown is kept once the server no longer says it: then the newest state shows, however briefly
/// each since has lasted.
pub const STALE: Duration = Duration::from_secs(1);

/// How pressing `state` is, by the server's precedence, or none for a state this build does not know.
pub fn rank(state: AgentState) -> Option<u8> {
    match state {
        AgentState::Idle => Some(0),
        AgentState::Thinking => Some(1),
        AgentState::Working => Some(2),
        AgentState::Speaking => Some(3),
        AgentState::Waiting => Some(4),
        AgentState::Listening => Some(5),
        AgentState::Error => Some(6),
        AgentState::Unknown => None,
    }
}

/// A less pressing state waiting to show, since when it was said, and since when the state shown has been superseded.
#[derive(Clone, Debug)]
struct Pending {
    status: AgentStatus,
    since: Instant,
    superseded: Instant,
}

/// The agent's state shown in a space, and a less pressing one waiting to show.
#[derive(Clone, Debug, Default)]
pub struct AgentView {
    /// The space it is shown for, whose sequence numbers it compares.
    space: Option<String>,
    shown: Option<(AgentStatus, u8)>,
    pending: Option<Pending>,
}

impl AgentView {
    /// The server says `status` in `space` at `now`, or nothing, or a state this build does not know, which shows
    /// nothing. Another space starts afresh.
    pub fn observe(&mut self, space: &str, status: Option<&AgentStatus>, now: Instant) {
        if self.space.as_deref() != Some(space) {
            *self = Self {
                space: Some(space.to_owned()),
                ..Self::default()
            };
        }
        let Some((status, pressing)) =
            status.and_then(|status| rank(status.state).map(|rank| (status, rank)))
        else {
            self.shown = None;
            self.pending = None;
            return;
        };
        match &self.shown {
            Some((shown, _)) if shown == status => self.pending = None,
            Some((shown, rank)) if shown.state != status.state && pressing < *rank => {
                let (since, superseded) = match &self.pending {
                    Some(pending) if pending.status.state == status.state => {
                        (pending.since, pending.superseded)
                    }
                    Some(pending) => (now, pending.superseded),
                    None => (now, now),
                };
                if now >= since + BRIEF || now >= superseded + STALE {
                    self.show(status, pressing);
                } else {
                    self.pending = Some(Pending {
                        status: status.clone(),
                        since,
                        superseded,
                    });
                }
            }
            _ => self.show(status, pressing),
        }
    }

    fn show(&mut self, status: &AgentStatus, rank: u8) {
        self.shown = Some((status.clone(), rank));
        self.pending = None;
    }

    /// The state shown in `space`, none if it was last told of another.
    pub fn shown(&self, space: &str) -> Option<&AgentStatus> {
        self.shown
            .as_ref()
            .filter(|_| self.space.as_deref() == Some(space))
            .map(|(status, _)| status)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(state: AgentState, sequence: u64) -> AgentStatus {
        AgentStatus {
            state,
            detail: None,
            sequence,
        }
    }

    fn shown(view: &AgentView) -> Option<AgentState> {
        view.shown("s").map(|status| status.state)
    }

    #[test]
    fn a_more_pressing_state_shows_at_once_and_a_less_pressing_one_once_it_lasts() {
        let start = Instant::now();
        let ms = Duration::from_millis;
        let mut view = AgentView::default();
        let say = |view: &mut AgentView, state, sequence, at| {
            view.observe("s", Some(&status(state, sequence)), start + ms(at));
        };
        say(&mut view, AgentState::Thinking, 1, 0);
        assert_eq!(shown(&view), Some(AgentState::Thinking));
        say(&mut view, AgentState::Speaking, 2, 0);
        assert_eq!(shown(&view), Some(AgentState::Speaking));
        // Working between two sentences, briefer than the hold, never shows.
        say(&mut view, AgentState::Working, 3, 100);
        say(&mut view, AgentState::Working, 3, 300);
        assert_eq!(shown(&view), Some(AgentState::Speaking));
        say(&mut view, AgentState::Speaking, 4, 400);
        assert_eq!(view.shown("s").unwrap().sequence, 4);
        // Idle shows once it has lasted, counted from when it was first said, through a change of its detail.
        say(&mut view, AgentState::Idle, 5, 500);
        say(&mut view, AgentState::Idle, 6, 800);
        assert_eq!(shown(&view), Some(AgentState::Speaking));
        say(&mut view, AgentState::Idle, 6, 900);
        assert_eq!(shown(&view), Some(AgentState::Idle));
        // The same state with another detail shows at once.
        view.observe(
            "s",
            Some(&AgentStatus {
                detail: Some("x".into()),
                ..status(AgentState::Idle, 7)
            }),
            start + ms(900),
        );
        assert_eq!(view.shown("s").unwrap().detail.as_deref(), Some("x"));
    }

    #[test]
    fn a_run_of_brief_less_pressing_states_shows_the_newest_once_the_shown_one_is_stale() {
        let start = Instant::now();
        let ms = Duration::from_millis;
        let mut view = AgentView::default();
        view.observe("s", Some(&status(AgentState::Waiting, 1)), start);
        // Each lasts less than the hold, so none shows on its own account.
        let chain = [
            (AgentState::Speaking, 300),
            (AgentState::Working, 600),
            (AgentState::Thinking, 900),
        ];
        for (sequence, (state, at)) in (2..).zip(chain) {
            view.observe("s", Some(&status(state, sequence)), start + ms(at));
            assert_eq!(shown(&view), Some(AgentState::Waiting), "{state:?}");
        }
        // The shown state was superseded a second ago: the newest shows.
        view.observe("s", Some(&status(AgentState::Idle, 5)), start + ms(1_300));
        assert_eq!(shown(&view), Some(AgentState::Idle));
    }

    #[test]
    fn another_space_starts_afresh_whatever_its_sequence_and_the_whole_status_is_compared() {
        let start = Instant::now();
        let mut view = AgentView::default();
        view.observe("a", Some(&status(AgentState::Speaking, 3)), start);
        // Moved straight into another space whose state has the same sequence: it shows at once.
        assert!(
            view.shown("b").is_none(),
            "nothing shown in a space it has not been told of"
        );
        view.observe("b", Some(&status(AgentState::Idle, 3)), start);
        assert_eq!(view.shown("b").unwrap().state, AgentState::Idle);
        // The same sequence with another state, as after a server restart, is a change.
        view.observe("b", Some(&status(AgentState::Error, 3)), start);
        assert_eq!(view.shown("b").unwrap().state, AgentState::Error);
        // And a less pressing one waiting in one space never shows in another.
        view.observe("b", Some(&status(AgentState::Thinking, 4)), start);
        view.observe("a", Some(&status(AgentState::Listening, 9)), start + BRIEF);
        assert_eq!(view.shown("a").unwrap().state, AgentState::Listening);
        view.observe(
            "a",
            Some(&status(AgentState::Listening, 9)),
            start + BRIEF * 3,
        );
        assert_eq!(view.shown("a").unwrap().sequence, 9);
    }

    #[test]
    fn nothing_or_an_unknown_state_shows_nothing() {
        let now = Instant::now();
        let mut view = AgentView::default();
        view.observe("s", Some(&status(AgentState::Error, 1)), now);
        view.observe("s", None, now);
        assert!(view.shown("s").is_none());
        view.observe("s", Some(&status(AgentState::Unknown, 2)), now);
        assert!(view.shown("s").is_none());
        assert_eq!(rank(AgentState::Unknown), None);
        assert!(rank(AgentState::Error) > rank(AgentState::Listening));
        assert!(rank(AgentState::Listening) > rank(AgentState::Waiting));
        assert!(rank(AgentState::Waiting) > rank(AgentState::Speaking));
        assert!(rank(AgentState::Speaking) > rank(AgentState::Working));
        assert!(rank(AgentState::Working) > rank(AgentState::Thinking));
        assert!(rank(AgentState::Thinking) > rank(AgentState::Idle));
    }
}
