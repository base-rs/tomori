// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Observable conversational activity, independent of the XR/audio backends.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[allow(dead_code)] // Produced on Quest; also exercised by desktop tests.
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
#[allow(dead_code)]
pub struct SpeechActivity {
    pub pending: bool,
    pub playing: bool,
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
#[allow(dead_code)]
pub struct Inputs {
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
#[allow(dead_code)]
pub struct Signals {
    ready_for: f32,
    /// How much longer the swarm speaks, once speech has stopped playing.
    speaking_for: f32,
    /// What was shown last.
    shown: Mode,
}

impl Signals {
    #[allow(dead_code)]
    pub fn update(&mut self, input: Inputs, dt: f32) -> Visual {
        let step = dt.clamp(0.0, 0.1);
        // Speaking follows speech playing on this device, rather than the session, which is often idle while its reply is still being heard.
        self.speaking_for = if input.speech.playing {
            SPEAKING_HOLD
        } else {
            (self.speaking_for - step).max(0.0)
        };
        let speaking = input.speech.playing || self.speaking_for > 0.0;
        let mode = if input.recording {
            Mode::Listening
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
            working: input.session == SessionActivity::Working,
            time: 0.0,
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
