// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Observable conversational activity, independent of the XR/audio backends.
use serde::{Deserialize, Serialize};
use tomori_protocol::AgentState;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SessionActivity {
    #[default]
    Unselected,
    Unknown,
    Working,
    Waiting,
    Ready,
    Unavailable,
}

/// Agent speech on this device: whether it is playing, live or played again, and how loud it is, even while muted locally, as the client's player measures it. See `design/speech.md`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SpeechActivity {
    pub pending: bool,
    pub playing: bool,
    /// Whether the server says more of the agent's speech is still to come, queued for synthesis or not yet all sent, so that the swarm speaks through the gaps while it is synthesized.
    pub to_come: bool,
    pub level: f32,
    pub failed: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    Neutral,
    Unknown,
    Working,
    Preparing,
    Speaking,
    Ready,
    Waiting,
    Listening,
    Transcribing,
    Unavailable,
}

/// All fields are observed status, never an estimate of hidden reasoning.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Visual {
    pub mode: Mode,
    pub level: f32,
    pub working: bool,
    pub time: f32,
}

impl Visual {
    pub fn color(self) -> [f32; 3] {
        match self.mode {
            Mode::Neutral => [0.3, 0.75, 1.0],
            Mode::Unknown => [0.3, 0.35, 0.48],
            Mode::Working => [0.38, 0.38, 1.0],
            Mode::Preparing => [0.65, 0.45, 0.95],
            Mode::Speaking => [0.15, 0.8, 1.0],
            Mode::Ready => [0.15, 0.95, 0.65],
            Mode::Waiting => [1.0, 0.65, 0.16],
            Mode::Listening => [0.25, 0.95, 1.0],
            Mode::Transcribing => [0.4, 0.65, 1.0],
            Mode::Unavailable => [1.0, 0.25, 0.15],
        }
    }

    /// Perceptual gain makes ordinary speech visible without changing telemetry.
    pub fn amplitude(self) -> f32 {
        (self.level * 6.0).clamp(0.0, 1.0).sqrt()
    }
}

#[derive(Default)]
pub struct Inputs {
    /// The agent's state as the server says it, smoothed, which the swarm shows in place of the session's activity and
    /// the speech still to come, where the server says one. See `design/agent-state.md`.
    pub agent: Option<AgentState>,
    pub session: SessionActivity,
    pub speech: SpeechActivity,
    pub recording: bool,
    pub voice_busy: bool,
    pub voice_error: bool,
    pub pending_text: bool,
}

/// How long the swarm keeps speaking after agent speech stops playing: longer than a buffer running dry for a moment, or the 400 ms breath between spoken segments, so that a reply is spoken throughout without flickering back to idle.
pub const SPEAKING_HOLD: f32 = 0.6;

/// How long the session must be ready before the swarm shows it.
const READY_SETTLE: f32 = 0.8;

/// A small settling interval avoids a one-frame ready flash at hand-offs, and a hold keeps the swarm speaking through the pauses in speech.
#[derive(Default)]
pub struct Signals {
    ready_for: f32,
    /// How much longer the swarm speaks, once speech has stopped playing.
    speaking_for: f32,
    /// What was shown last.
    shown: Mode,
}

impl Signals {
    pub fn update(&mut self, input: Inputs, dt: f32) -> Visual {
        let step = dt.clamp(0.0, 0.1);
        // Speaking follows speech playing on this device, rather than the session, which is often idle while its reply is still being heard.
        self.speaking_for = if input.speech.playing {
            SPEAKING_HOLD
        } else {
            (self.speaking_for - step).max(0.0)
        };
        // And while more of it is still to come, however long the next of it takes to synthesize, as the server's state or else its speech still to come says, unless a reply is held for the participant, which the swarm prepares for.
        let agent = input.agent.filter(|agent| *agent != AgentState::Unknown);
        let to_come = match agent {
            Some(agent) => agent == AgentState::Speaking,
            None => input.speech.to_come,
        };
        let speaking =
            input.speech.playing || self.speaking_for > 0.0 || (to_come && !input.speech.pending);
        let mode = if input.recording {
            Mode::Listening
        } else if let Some(agent) = agent {
            Self::agent_mode(agent, speaking, &input)
        } else if speaking {
            Mode::Speaking
        } else if input.voice_busy {
            Mode::Transcribing
        } else if input.voice_error
            || input.speech.failed
            || input.session == SessionActivity::Unavailable
        {
            Mode::Unavailable
        } else if input.pending_text || input.speech.pending {
            Mode::Preparing
        } else {
            match input.session {
                SessionActivity::Unselected => Mode::Neutral,
                SessionActivity::Unknown => Mode::Unknown,
                SessionActivity::Working => Mode::Working,
                SessionActivity::Waiting => Mode::Waiting,
                SessionActivity::Ready => Mode::Ready,
                SessionActivity::Unavailable => {
                    unreachable!("an unavailable session is handled above")
                }
            }
        };
        self.ready_for = match mode {
            // Speech held across its pauses has settled already, so a reply that ends ready shows ready at once.
            Mode::Ready if self.shown == Mode::Speaking => READY_SETTLE,
            Mode::Ready => self.ready_for + step,
            _ => 0.0,
        };
        let shown = if mode == Mode::Ready && self.ready_for < READY_SETTLE {
            Mode::Preparing
        } else {
            mode
        };
        self.shown = shown;
        Visual {
            mode: shown,
            level: if mode == Mode::Speaking && input.speech.level.is_finite() {
                input.speech.level.clamp(0.0, 1.0)
            } else {
                0.0
            },
            working: match agent {
                Some(agent) => matches!(agent, AgentState::Working | AgentState::Thinking),
                None => input.session == SessionActivity::Working,
            },
            time: 0.0,
        }
    }

    /// What the swarm shows of the server's agent state `agent`, by its precedence, with this device's own facts
    /// layered on it: speech playing here, `speaking`, below listening and waiting, and its own transcription, failures
    /// and held reply below that. See `design/agent-state.md`.
    fn agent_mode(agent: AgentState, speaking: bool, input: &Inputs) -> Mode {
        match agent {
            AgentState::Error => Mode::Unavailable,
            AgentState::Listening => Mode::Listening,
            AgentState::Waiting => Mode::Waiting,
            _ if speaking => Mode::Speaking,
            _ if input.voice_busy => Mode::Transcribing,
            _ if input.voice_error || input.speech.failed => Mode::Unavailable,
            // Speech still to come for a reply held for the participant prepares, as the reply does.
            _ if input.pending_text || input.speech.pending || agent == AgentState::Speaking => {
                Mode::Preparing
            }
            AgentState::Working | AgentState::Thinking => Mode::Working,
            _ => Mode::Ready,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transcribing_and_an_unavailable_session_show_as_such() {
        let mut signals = Signals::default();
        let busy = Inputs {
            voice_busy: true,
            ..Default::default()
        };
        assert_eq!(signals.update(busy, 0.1).mode, Mode::Transcribing);
        let unavailable = Inputs {
            session: SessionActivity::Unavailable,
            ..Default::default()
        };
        assert_eq!(signals.update(unavailable, 0.1).mode, Mode::Unavailable);
    }

    /// Inputs for a session in `session`, with agent speech `playing` at `level`.
    fn speech(session: SessionActivity, playing: bool, level: f32) -> Inputs {
        Inputs {
            session,
            speech: SpeechActivity {
                playing,
                level,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    /// The modes shown over `seconds` of frames of `dt`, all with `input`.
    fn modes(
        signals: &mut Signals,
        input: impl Fn() -> Inputs,
        seconds: f32,
        dt: f32,
    ) -> Vec<Mode> {
        (0..(seconds / dt).round() as usize)
            .map(|_| signals.update(input(), dt).mode)
            .collect()
    }

    #[test]
    fn queued_speech_never_looks_ready() {
        let mut signals = Signals::default();
        let visual = signals.update(
            Inputs {
                session: SessionActivity::Ready,
                speech: SpeechActivity {
                    pending: true,
                    ..Default::default()
                },
                ..Default::default()
            },
            0.02,
        );
        assert_eq!(visual.mode, Mode::Preparing);
        for _ in 0..7 {
            assert_eq!(
                signals
                    .update(speech(SessionActivity::Ready, false, 0.0), 0.1)
                    .mode,
                Mode::Preparing
            );
        }
        assert_eq!(
            signals
                .update(speech(SessionActivity::Ready, false, 0.0), 0.1)
                .mode,
            Mode::Ready
        );
    }

    #[test]
    fn speech_starting_shows_speaking_with_its_level() {
        let mut signals = Signals::default();
        assert_eq!(
            signals
                .update(speech(SessionActivity::Working, false, 0.0), 0.1)
                .mode,
            Mode::Working
        );
        let visual = signals.update(speech(SessionActivity::Working, true, 0.4), 1.0 / 72.0);
        assert_eq!(visual.mode, Mode::Speaking);
        assert_eq!(visual.level, 0.4);
        assert!(visual.working);
    }

    #[test]
    fn breaths_and_gaps_between_segments_keep_speaking() {
        let mut signals = Signals::default();
        let dt = 1.0 / 72.0;
        for _ in 0..3 {
            // A segment, then a breath, or a buffer running dry, of up to the hold.
            let segment = modes(
                &mut signals,
                || speech(SessionActivity::Ready, true, 0.5),
                1.5,
                dt,
            );
            let breath = modes(
                &mut signals,
                || speech(SessionActivity::Ready, false, 0.0),
                SPEAKING_HOLD - 0.05,
                dt,
            );
            assert!(
                segment
                    .iter()
                    .chain(&breath)
                    .all(|mode| *mode == Mode::Speaking)
            );
        }
        // The level falls with the speech while held, rather than holding the last.
        assert_eq!(
            signals
                .update(speech(SessionActivity::Ready, false, 0.0), dt)
                .level,
            0.0
        );
    }

    #[test]
    fn the_server_s_agent_state_shows_by_its_precedence_with_this_device_s_speech_layered_on_it() {
        use AgentState as A;
        let shown = |agent: AgentState, change: &dyn Fn(&mut Inputs)| {
            let mut signals = Signals::default();
            let input = || {
                let mut input = Inputs {
                    agent: Some(agent),
                    // What an older server's session says, which the server's state replaces.
                    session: SessionActivity::Ready,
                    ..Default::default()
                };
                change(&mut input);
                input
            };
            // Long enough for ready to settle.
            let visual = (0..10)
                .map(|_| signals.update(input(), 0.1))
                .last()
                .unwrap();
            (visual.mode, visual.working)
        };
        let nothing = |_: &mut Inputs| {};
        let playing = |input: &mut Inputs| input.speech.playing = true;
        let held = |input: &mut Inputs| input.speech.pending = true;
        assert_eq!(shown(A::Thinking, &nothing), (Mode::Working, true));
        assert_eq!(shown(A::Working, &nothing), (Mode::Working, true));
        assert_eq!(shown(A::Speaking, &nothing), (Mode::Speaking, false));
        assert_eq!(shown(A::Waiting, &nothing), (Mode::Waiting, false));
        assert_eq!(shown(A::Listening, &nothing), (Mode::Listening, false));
        assert_eq!(shown(A::Error, &nothing), (Mode::Unavailable, false));
        // Idle settles to ready as the session's ready does.
        assert_eq!(shown(A::Idle, &nothing), (Mode::Ready, false));
        // Speech playing here shows over thinking, working and idle, and not over waiting, listening or an error.
        assert_eq!(shown(A::Working, &playing).0, Mode::Speaking);
        assert_eq!(shown(A::Idle, &playing).0, Mode::Speaking);
        assert_eq!(shown(A::Waiting, &playing).0, Mode::Waiting);
        assert_eq!(shown(A::Listening, &playing).0, Mode::Listening);
        assert_eq!(shown(A::Error, &playing).0, Mode::Unavailable);
        // Speech to come for a reply held here prepares; this device's own recording, transcription and failures show.
        assert_eq!(shown(A::Speaking, &held).0, Mode::Preparing);
        assert_eq!(shown(A::Idle, &held).0, Mode::Preparing);
        assert_eq!(
            shown(A::Waiting, &|input| input.recording = true).0,
            Mode::Listening
        );
        assert_eq!(
            shown(A::Thinking, &|input| input.voice_busy = true).0,
            Mode::Transcribing
        );
        assert_eq!(
            shown(A::Thinking, &|input| input.speech.failed = true).0,
            Mode::Unavailable
        );
        // Speech still to come that an older server says is not used where the server says its state.
        assert_eq!(
            shown(A::Idle, &|input| input.speech.to_come = true).0,
            Mode::Ready
        );
        // A state this build does not know shows the session's activity, as an older server's.
        assert_eq!(
            shown(A::Unknown, &|input| input.session =
                SessionActivity::Working),
            (Mode::Working, true)
        );
    }

    #[test]
    fn a_gap_while_more_is_to_come_keeps_speaking_however_long_but_a_held_reply_prepares() {
        let mut signals = Signals::default();
        let dt = 0.05;
        let to_come = |pending: bool| Inputs {
            session: SessionActivity::Ready,
            speech: SpeechActivity {
                to_come: true,
                pending,
                ..Default::default()
            },
            ..Default::default()
        };
        modes(
            &mut signals,
            || speech(SessionActivity::Working, true, 0.5),
            1.0,
            dt,
        );
        // The next sentence takes far longer than the hold to synthesize, the session ready meanwhile.
        let gap = modes(&mut signals, || to_come(false), SPEAKING_HOLD * 10.0, dt);
        assert!(gap.iter().all(|mode| *mode == Mode::Speaking));
        assert_eq!(signals.update(to_come(false), dt).level, 0.0);
        // A reply held for the participant prepares, as it does without speech to come.
        let held = modes(&mut signals, || to_come(true), SPEAKING_HOLD * 2.0, dt);
        assert_eq!(held.last(), Some(&Mode::Preparing));
    }

    #[test]
    fn playback_ending_returns_to_idle_after_the_hold() {
        let mut signals = Signals::default();
        let dt = 0.05;
        modes(
            &mut signals,
            || speech(SessionActivity::Ready, true, 0.5),
            2.0,
            dt,
        );
        let after = modes(
            &mut signals,
            || speech(SessionActivity::Ready, false, 0.0),
            1.0,
            dt,
        );
        let held = after
            .iter()
            .take_while(|mode| **mode == Mode::Speaking)
            .count();
        // About the hold, whichever way its last frame rounds.
        let hold = (SPEAKING_HOLD / dt).round() as usize;
        assert!((hold - 2..=hold).contains(&held), "held for {held} frames");
        // Straight to ready, without settling through preparing again.
        assert!(after[held..].iter().all(|mode| *mode == Mode::Ready));
        // A session that is not ready shows as it is, once speech has ended.
        modes(
            &mut signals,
            || speech(SessionActivity::Ready, true, 0.5),
            0.5,
            dt,
        );
        let after = modes(
            &mut signals,
            || speech(SessionActivity::Waiting, false, 0.0),
            1.0,
            dt,
        );
        assert_eq!(after.last(), Some(&Mode::Waiting));
    }

    #[test]
    fn speech_playing_wins_over_the_session_whatever_it_is_doing() {
        for session in [
            SessionActivity::Ready,
            SessionActivity::Working,
            SessionActivity::Waiting,
            SessionActivity::Unknown,
            SessionActivity::Unselected,
        ] {
            let mut signals = Signals::default();
            assert!(
                modes(&mut signals, || speech(session, true, 0.3), 3.0, 0.1)
                    .iter()
                    .all(|mode| *mode == Mode::Speaking),
                "{session:?}"
            );
        }
        // Recording still shows listening, over speech.
        let mut signals = Signals::default();
        let mut input = speech(SessionActivity::Ready, true, 0.3);
        input.recording = true;
        let visual = signals.update(input, 0.1);
        assert_eq!(visual.mode, Mode::Listening);
        assert_eq!(visual.level, 0.0);
    }

    #[test]
    fn an_unmeasurable_level_is_silent() {
        let visual = Signals::default().update(speech(SessionActivity::Ready, true, f32::NAN), 0.1);
        assert_eq!(visual.mode, Mode::Speaking);
        assert_eq!(visual.level, 0.0);
    }

    #[test]
    fn waits_errors_and_uncertainty_are_not_ready() {
        for (session, expected) in [
            (SessionActivity::Waiting, Mode::Waiting),
            (SessionActivity::Unknown, Mode::Unknown),
            (SessionActivity::Unavailable, Mode::Unavailable),
        ] {
            assert_eq!(
                Signals::default()
                    .update(
                        Inputs {
                            session,
                            ..Default::default()
                        },
                        5.0
                    )
                    .mode,
                expected
            );
        }
    }

    #[test]
    fn recording_and_speech_are_distinct_from_work() {
        let mut signals = Signals::default();
        let visual = signals.update(
            Inputs {
                session: SessionActivity::Working,
                recording: true,
                ..Default::default()
            },
            0.01,
        );
        assert_eq!(visual.mode, Mode::Listening);
        assert!(visual.working);
        let visual = signals.update(
            Inputs {
                session: SessionActivity::Working,
                speech: SpeechActivity {
                    playing: true,
                    level: 0.7,
                    ..Default::default()
                },
                ..Default::default()
            },
            0.01,
        );
        assert_eq!(visual.mode, Mode::Speaking);
        assert!(visual.working);
        assert_eq!(visual.level, 0.7);
    }
}
