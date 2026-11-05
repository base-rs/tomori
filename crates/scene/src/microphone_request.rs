// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! When the headset asks for the microphone, ahead of the first recording, and how it follows the answer without waiting for it. See `design/arrival.md`.
//!
//! The platform's dialog is a system panel shown over the immersive app. Asking as the session first gained focus put it up before the app had shown a single frame, while the system was still settling the launch, and the participant could be left in a black world. Asking is a call that returns at once, so it never stopped frames; what matters is when it is made. So the request waits until the scene has been shown with focus for [`SETTLE`]. Android answers later, with no callback that a native activity receives, so the frame loop checks again every [`CHECK`] until the microphone is allowed or the request has evidently been answered without allowing it; those checks only decide what the participant is told and what is logged, and the frame loop never waits for them.

use std::time::{Duration, Instant};

/// How long the scene must have been shown, with focus, before the request is made, so that the dialog appears over a world already drawn.
pub const SETTLE: Duration = Duration::from_secs(1);
/// How often to check again while the request is unanswered.
pub const CHECK: Duration = Duration::from_millis(500);
/// How long focus must have been back, after the dialog took it, before a microphone still not allowed counts as declined.
pub const RETURNED: Duration = Duration::from_secs(1);
/// How long, with focus never lost, before a microphone still not allowed counts as declined: Android shows no dialog once the participant has told it not to ask again.
pub const UNSHOWN: Duration = Duration::from_secs(10);

/// What an answer from the platform means.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Answer {
    /// The request was made, and is waiting for the participant.
    Asked,
    /// Still unanswered.
    Waiting,
    /// The microphone is allowed.
    Allowed,
    /// Answered without allowing it; the first recording asks again.
    Declined,
    /// The platform could not be asked.
    Failed(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    /// Not asked yet; since when the scene has been shown with focus.
    Settling {
        since: Option<Instant>,
    },
    /// Asked, and not answered yet.
    Waiting {
        /// Since when the session has had focus again, if it has it.
        focused_since: Option<Instant>,
        /// Whether focus was lost since the request, as the dialog takes it.
        left: bool,
        checked: Instant,
    },
    Done,
}

/// The request for the microphone, from asking to its answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MicrophoneRequest {
    state: State,
}

impl Default for MicrophoneRequest {
    fn default() -> Self {
        Self {
            state: State::Settling { since: None },
        }
    }
}

impl MicrophoneRequest {
    /// After a turn of the frame loop at `now`, with the session `focused` or not, and the scene `presented` in a frame or not. Returns whether to ask the platform now: the first time, that makes the request; after that, it only checks.
    pub fn step(&mut self, focused: bool, presented: bool, now: Instant) -> bool {
        match &mut self.state {
            State::Settling { since } => {
                if !(focused && presented) {
                    *since = None;
                    return false;
                }
                now.duration_since(*since.get_or_insert(now)) >= SETTLE
            }
            State::Waiting {
                focused_since,
                left,
                checked,
            } => {
                if focused {
                    focused_since.get_or_insert(now);
                } else {
                    *focused_since = None;
                    *left = true;
                }
                let due = now.duration_since(*checked) >= CHECK;
                if due {
                    *checked = now;
                }
                due
            }
            State::Done => false,
        }
    }

    /// The session lost focus, as the dialog takes it, even if it gains it again before the next turn of the frame loop.
    pub fn blur(&mut self) {
        match &mut self.state {
            State::Settling { since } => *since = None,
            State::Waiting {
                focused_since,
                left,
                ..
            } => {
                *focused_since = None;
                *left = true;
            }
            State::Done => {}
        }
    }

    /// The platform answered at `now` whether the microphone is allowed.
    pub fn answered(&mut self, allowed: Result<bool, String>, now: Instant) -> Answer {
        let answer = match (allowed, self.state) {
            (Err(error), _) => Answer::Failed(error),
            (Ok(true), _) => Answer::Allowed,
            (Ok(false), State::Settling { .. }) => {
                self.state = State::Waiting {
                    focused_since: Some(now),
                    left: false,
                    checked: now,
                };
                return Answer::Asked;
            }
            (
                Ok(false),
                State::Waiting {
                    focused_since,
                    left,
                    ..
                },
            ) => {
                let limit = if left { RETURNED } else { UNSHOWN };
                if !focused_since.is_some_and(|since| now.duration_since(since) >= limit) {
                    return Answer::Waiting;
                }
                Answer::Declined
            }
            (Ok(false), State::Done) => Answer::Declined,
        };
        self.state = State::Done;
        answer
    }

    /// Whether the request has been made and is unanswered.
    pub fn waiting(&self) -> bool {
        matches!(self.state, State::Waiting { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run `request` for `turns` turns of `step` apart from `now`, returning the time after them and whether any asked.
    fn run(
        request: &mut MicrophoneRequest,
        now: Instant,
        turns: u32,
        step: Duration,
        focused: bool,
        presented: bool,
    ) -> (Instant, bool) {
        let mut asked = false;
        let mut now = now;
        for _ in 0..turns {
            now += step;
            asked |= request.step(focused, presented, now);
        }
        (now, asked)
    }

    const FRAME: Duration = Duration::from_millis(14);

    /// A request that has just been made at `now`.
    fn asked(now: Instant) -> MicrophoneRequest {
        let mut request = MicrophoneRequest::default();
        assert!(!request.step(true, true, now));
        assert!(request.step(true, true, now + SETTLE));
        assert_eq!(request.answered(Ok(false), now + SETTLE), Answer::Asked);
        assert!(request.waiting());
        request
    }

    #[test]
    fn it_waits_until_the_scene_has_been_shown_with_focus_for_a_while() {
        let start = Instant::now();
        let mut request = MicrophoneRequest::default();
        // Focused, but nothing shown yet: the launch is still settling.
        let (now, early) = run(&mut request, start, 200, FRAME, true, false);
        assert!(!early);
        // Shown, but without focus, as while a system panel is up.
        let (now, early) = run(&mut request, now, 200, FRAME, false, true);
        assert!(!early);
        // Shown with focus, but not yet for long enough.
        let (now, early) = run(&mut request, now, 50, FRAME, true, true);
        assert!(!early);
        // A turn without a frame starts the wait again.
        assert!(!request.step(true, false, now));
        let (now, early) = run(&mut request, now, 50, FRAME, true, true);
        assert!(!early);
        let (_, asked) = run(&mut request, now, 30, FRAME, true, true);
        assert!(asked && !request.waiting());
    }

    #[test]
    fn already_allowed_needs_no_dialog() {
        let start = Instant::now();
        let mut request = MicrophoneRequest::default();
        request.step(true, true, start);
        assert!(request.step(true, true, start + SETTLE));
        assert_eq!(request.answered(Ok(true), start + SETTLE), Answer::Allowed);
        assert!(!request.waiting());
        let (_, again) = run(&mut request, start + SETTLE, 200, FRAME, true, true);
        assert!(!again);
    }

    #[test]
    fn allowed_in_the_dialog_while_frames_go_on() {
        let start = Instant::now();
        let mut request = asked(start);
        let now = start + SETTLE;
        // The dialog takes focus; the scene is still shown, and checked every half second without waiting.
        assert!(!request.step(false, true, now + FRAME));
        assert!(request.step(false, true, now + CHECK));
        assert_eq!(request.answered(Ok(false), now + CHECK), Answer::Waiting);
        assert!(!request.step(false, true, now + CHECK + FRAME));
        // The session stops and starts again while it is up.
        assert!(!request.step(false, false, now + CHECK * 2 - FRAME));
        assert!(request.step(false, false, now + CHECK * 2));
        assert_eq!(
            request.answered(Ok(false), now + CHECK * 2),
            Answer::Waiting
        );
        // Allowed: focus returns, and the next check sees it.
        assert!(request.step(true, true, now + CHECK * 3));
        assert_eq!(request.answered(Ok(true), now + CHECK * 3), Answer::Allowed);
        assert!(!request.waiting());
        assert!(!request.step(true, true, now + CHECK * 4));
    }

    #[test]
    fn denied_in_the_dialog_once_focus_has_been_back_a_moment() {
        let start = Instant::now();
        let mut request = asked(start);
        let now = start + SETTLE;
        assert!(request.step(false, true, now + CHECK));
        assert_eq!(request.answered(Ok(false), now + CHECK), Answer::Waiting);
        // Focus returns, but not yet for long enough to be sure the dialog has gone.
        let returned = now + CHECK * 2;
        assert!(request.step(true, true, returned));
        assert_eq!(request.answered(Ok(false), returned), Answer::Waiting);
        assert!(request.step(true, true, returned + RETURNED));
        assert_eq!(
            request.answered(Ok(false), returned + RETURNED),
            Answer::Declined
        );
        assert!(!request.waiting());
        assert!(!request.step(true, true, returned + RETURNED * 2));
    }

    #[test]
    fn focus_lost_and_regained_between_turns_still_counts() {
        let start = Instant::now();
        let mut request = MicrophoneRequest::default();
        request.step(true, true, start);
        // Before the request, it starts the wait again.
        request.blur();
        assert!(!request.step(true, true, start + SETTLE));
        assert!(request.step(true, true, start + SETTLE * 2));
        let now = start + SETTLE * 2;
        assert_eq!(request.answered(Ok(false), now), Answer::Asked);
        // After it, the dialog came and went between two turns.
        request.blur();
        assert!(request.step(true, true, now + CHECK));
        assert_eq!(request.answered(Ok(false), now + CHECK), Answer::Waiting);
        assert!(request.step(true, true, now + CHECK + RETURNED));
        assert_eq!(
            request.answered(Ok(false), now + CHECK + RETURNED),
            Answer::Declined
        );
        // Once answered, it changes nothing.
        request.blur();
        assert!(!request.waiting());
    }

    #[test]
    fn no_dialog_shown_counts_as_declined_after_a_while() {
        let start = Instant::now();
        let mut request = asked(start);
        let now = start + SETTLE;
        assert!(request.step(true, true, now + CHECK));
        assert_eq!(request.answered(Ok(false), now + CHECK), Answer::Waiting);
        assert!(request.step(true, true, now + UNSHOWN));
        assert_eq!(request.answered(Ok(false), now + UNSHOWN), Answer::Declined);
    }

    #[test]
    fn a_platform_that_cannot_be_asked_fails_once() {
        let start = Instant::now();
        let mut request = MicrophoneRequest::default();
        request.step(true, true, start);
        assert!(request.step(true, true, start + SETTLE));
        assert_eq!(
            request.answered(Err("no activity".into()), start + SETTLE),
            Answer::Failed("no activity".into())
        );
        assert!(!request.step(true, true, start + SETTLE * 2));
        // An answer after it is done changes nothing.
        assert_eq!(
            request.answered(Ok(false), start + SETTLE * 2),
            Answer::Declined
        );
        assert!(!request.waiting());
    }
}
