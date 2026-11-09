// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Whose turn it is to speak, on this device: whether the agent is still speaking, as the participant hears it, when it has finished a reply they heard so that it is their turn, and the rest of what it was saying skipped when they speak over it. Pure, and driven by the frame's time, so that it is tested deterministically. See `design/speech.md` and `design/interaction.md`.
use std::time::{Duration, Instant};

/// How long the agent still counts as speaking once its speech stops playing: the swarm's hold, so that the pill and the swarm agree, longer than the breath between spoken segments.
pub const HOLD: Duration =
    Duration::from_millis((crate::interaction::SPEAKING_HOLD * 1000.0) as u64);

/// How long the pill's dot pulses once it is the participant's turn.
pub const PULSE: Duration = Duration::from_millis(900);

/// How agent speech that is playing on this device is heard here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Speaking {
    /// Heard, live or played again.
    Heard,
    /// Muted locally by this participant, while the others still hear it.
    Muted,
    /// Skipped: this participant spoke over it, so the rest of what the agent was saying is silent here until it says something else.
    Skipped,
}

#[derive(Debug, Default)]
pub struct Turn {
    /// The frame's time, from the connection's tick, which agent speech is reported after.
    now: Option<Instant>,
    /// When agent speech last played here, heard or not.
    played: Option<Instant>,
    /// Whether agent speech was heard here since the participant last spoke or was last told it was their turn.
    heard: bool,
    /// When it last became the participant's turn.
    turn: Option<Instant>,
    /// The message of the agent's speech announced last, live or played again.
    announced: Option<String>,
    /// While the agent's speech is skipped, the message it was saying when the participant spoke over it, if one was announced.
    skipped: Option<Option<String>>,
    /// Whether the server holds a reply for the participant, while they record or in the pause after they let go of X.
    held: bool,
    /// When the server last said it no longer holds it, until it plays, so that it is still waiting until it is heard.
    unheld: Option<Instant>,
}

impl Turn {
    /// The frame's time, given before the frame's speech.
    pub fn tick(&mut self, now: Instant) {
        self.now = Some(now);
    }

    /// The frame's time, as last given.
    pub fn now(&self) -> Option<Instant> {
        self.now
    }

    /// Whether agent speech is playing on this device this frame, heard or not, and whether it is `audible`: neither muted nor skipped, nor silenced while the participant records.
    pub fn played(&mut self, playing: bool, audible: bool) {
        if playing {
            self.played = Some(self.now.unwrap_or_else(Instant::now));
            self.heard |= audible;
            self.unheld = None;
        }
    }

    /// Whether the agent is speaking on this device at `now`: its speech playing, or having played within [`HOLD`], as the swarm shows it.
    pub fn speaking(&self, now: Instant) -> bool {
        self.played
            .is_some_and(|played| now.saturating_duration_since(played) < HOLD)
    }

    /// The agent announced speech of `message`, live or played again: returns whether that ends a skip, as it says something other than what the participant spoke over.
    pub fn announced(&mut self, message: &str) -> bool {
        self.announced = Some(message.to_owned());
        let ends = self
            .skipped
            .as_ref()
            .is_some_and(|skipped| skipped.as_deref() != Some(message));
        if ends {
            self.skipped = None;
        }
        ends
    }

    /// The server said whether it holds a reply for the participant, at the frame's time. See `design/speech.md`.
    pub fn held(&mut self, held: bool) {
        if held {
            self.unheld = None;
        } else if self.held {
            self.unheld = Some(self.now.unwrap_or_else(Instant::now));
        }
        self.held = held;
    }

    /// Whether a reply is waiting for the participant at `now`: held by the server, or no longer held but not yet
    /// playing, for up to [`HOLD`], the time it takes to reach this device, so that the pill goes straight from waiting
    /// to speaking.
    pub fn waiting(&self, now: Instant) -> bool {
        self.held
            || self
                .unheld
                .is_some_and(|unheld| now.saturating_duration_since(unheld) < HOLD)
    }

    /// The participant started speaking at `now`: it is no longer a turn to cue, and speech that has not been heard since is not one either. Returns whether the agent was speaking, in which case the rest of what it is saying is skipped.
    pub fn speak(&mut self, now: Instant) -> bool {
        self.heard = false;
        self.turn = None;
        let speaking = self.speaking(now);
        if speaking {
            self.skipped = Some(self.announced.clone());
        }
        speaking
    }

    /// Hear the agent again: the participant asked for a reply played again, or left the space or lost the connection, after which nothing is skipped. Returns whether something was skipped.
    pub fn resume(&mut self) -> bool {
        self.skipped.take().is_some()
    }

    pub fn is_skipped(&self) -> bool {
        self.skipped.is_some()
    }

    /// Forget everything, as on leaving a space or losing the connection, after which nothing said before is the participant's turn.
    pub fn reset(&mut self) {
        *self = Self {
            now: self.now,
            ..Self::default()
        };
    }

    /// At `now`, the agent finished speaking a reply heard here, if it is no longer speaking and `free`: idle, with nothing waiting for the participant or on its way from them. Then it is their turn, once.
    pub fn settle(&mut self, now: Instant, free: bool) {
        if self.heard && free && !self.speaking(now) && !self.waiting(now) {
            self.heard = false;
            self.turn = Some(now);
        }
    }

    /// When it last became the participant's turn, until they speak.
    pub fn turn(&self) -> Option<Instant> {
        self.turn
    }

    /// How brightly the pill's dot pulses at `now`, from 0 to 1: rising and falling once over [`PULSE`] after it became the participant's turn, and 0 otherwise.
    pub fn pulse(&self, now: Instant) -> f32 {
        let Some(turn) = self.turn else {
            return 0.0;
        };
        let elapsed = now.saturating_duration_since(turn);
        if elapsed >= PULSE {
            return 0.0;
        }
        (std::f32::consts::PI * elapsed.as_secs_f32() / PULSE.as_secs_f32()).sin()
    }

    /// How agent speech playing at `now` is heard here, given whether it is `muted` locally, or `None` while the agent is not speaking.
    pub fn speaking_as(&self, now: Instant, muted: bool) -> Option<Speaking> {
        if !self.speaking(now) {
            None
        } else if muted {
            Some(Speaking::Muted)
        } else if self.is_skipped() {
            Some(Speaking::Skipped)
        } else {
            Some(Speaking::Heard)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(milliseconds: u64) -> Duration {
        Duration::from_millis(milliseconds)
    }

    /// A frame at `at`, with speech `playing`, heard if `audible`.
    fn frame(turn: &mut Turn, at: Instant, playing: bool, audible: bool) {
        turn.tick(at);
        turn.played(playing, audible);
    }

    #[test]
    fn speaking_holds_across_breaths_and_ends_after_the_hold() {
        let start = Instant::now();
        let mut turn = Turn::default();
        assert!(!turn.speaking(start));
        frame(&mut turn, start, true, true);
        assert!(turn.speaking(start));
        // A breath shorter than the hold.
        assert!(turn.speaking(start + ms(400)));
        assert!(!turn.speaking(start + HOLD));
        assert_eq!(turn.speaking_as(start, false), Some(Speaking::Heard));
        assert_eq!(turn.speaking_as(start, true), Some(Speaking::Muted));
        assert_eq!(turn.speaking_as(start + HOLD, false), None);
    }

    #[test]
    fn the_turn_comes_once_after_speech_heard_ends_and_the_agent_is_free() {
        let start = Instant::now();
        let mut turn = Turn::default();
        // Nothing heard: an agent idle after a reply with no speech is not a turn.
        turn.settle(start, true);
        assert_eq!(turn.turn(), None);
        frame(&mut turn, start, true, true);
        // Still speaking, though the agent is idle on the server.
        turn.settle(start + ms(100), true);
        assert_eq!(turn.turn(), None);
        // Speech ended, but the agent is still working.
        let ended = start + HOLD;
        turn.settle(ended, false);
        assert_eq!(turn.turn(), None);
        // Free at last: the participant's turn, once.
        let free = ended + ms(500);
        turn.settle(free, true);
        assert_eq!(turn.turn(), Some(free));
        turn.settle(free + ms(20), true);
        assert_eq!(turn.turn(), Some(free));
        // The dot pulses once, rising and falling.
        assert_eq!(turn.pulse(free), 0.0);
        assert!(turn.pulse(free + PULSE / 2) > 0.99);
        assert_eq!(turn.pulse(free + PULSE), 0.0);
        // Speaking takes the turn.
        assert!(!turn.speak(free + ms(2_000)));
        assert_eq!(turn.turn(), None);
        assert_eq!(turn.pulse(free + ms(2_000)), 0.0);
        turn.settle(free + ms(3_000), true);
        assert_eq!(turn.turn(), None);
    }

    #[test]
    fn speech_not_heard_here_is_no_turn() {
        let start = Instant::now();
        let mut turn = Turn::default();
        // Muted, skipped or silenced while recording.
        frame(&mut turn, start, true, false);
        assert!(turn.speaking(start));
        turn.settle(start + HOLD, true);
        assert_eq!(turn.turn(), None);
    }

    #[test]
    fn speaking_over_the_agent_skips_what_it_was_saying_until_it_says_something_else() {
        let start = Instant::now();
        let mut turn = Turn::default();
        assert!(!turn.announced("m1"));
        frame(&mut turn, start, true, true);
        // Within a breath of it still counts as speaking over it.
        assert!(turn.speak(start + ms(300)));
        assert!(turn.is_skipped());
        assert_eq!(
            turn.speaking_as(start + ms(300), false),
            Some(Speaking::Skipped)
        );
        // A mute shows over the skip.
        assert_eq!(
            turn.speaking_as(start + ms(300), true),
            Some(Speaking::Muted)
        );
        // More of the same message stays skipped; the next one is heard.
        assert!(!turn.announced("m1"));
        assert!(turn.is_skipped());
        assert!(turn.announced("m2"));
        assert!(!turn.is_skipped());
        // What was skipped is no turn of the participant's, though it played.
        frame(&mut turn, start + ms(400), true, false);
        turn.settle(start + ms(2_000), true);
        assert_eq!(turn.turn(), None);
    }

    #[test]
    fn speaking_over_speech_never_announced_skips_until_any_is_and_a_replay_resumes_it() {
        let start = Instant::now();
        let mut turn = Turn::default();
        frame(&mut turn, start, true, true);
        assert!(turn.speak(start));
        assert!(turn.announced("m1"));
        frame(&mut turn, start + ms(100), true, true);
        assert!(turn.speak(start + ms(100)));
        assert!(turn.resume());
        assert!(!turn.resume());
        // Speaking long after speech ended skips nothing.
        assert!(!turn.speak(start + ms(5_000)));
        assert!(!turn.is_skipped());
    }

    #[test]
    fn held_speech_waiting_out_the_pause_after_x_is_let_go_shows_neither_speaking_nor_a_turn() {
        // The server holds a reply that starts while the participant records, and starts it 600 ms after they let go
        // of X, its `RELEASE_PAUSE`, sending nothing meanwhile; the pill follows what plays here.
        let pause = ms(600);
        let start = Instant::now();
        let mut turn = Turn::default();
        assert!(!turn.announced("m1"));
        frame(&mut turn, start, true, true);
        // They speak over the end of the first message, and the next reply is held for them while they record.
        let pressed = start + ms(100);
        assert!(turn.speak(pressed));
        frame(&mut turn, pressed, true, false);
        let released = pressed + ms(5_000);
        // The server says it holds the next reply for them.
        assert!(!turn.waiting(pressed));
        turn.held(true);
        for at in [pressed + ms(1_000), released] {
            frame(&mut turn, at, false, false);
            assert!(turn.waiting(at));
        }
        // Through the pause: nothing plays, so the agent is not speaking, and, though it is free, it is not their turn,
        // since they spoke last; a reply is waiting.
        for at in [released, released + pause / 2, released + pause - ms(1)] {
            frame(&mut turn, at, false, false);
            assert_eq!(turn.speaking_as(at, false), None);
            assert!(turn.waiting(at));
            turn.settle(at, true);
            assert_eq!(turn.turn(), None);
        }
        // Once the pause has passed, the server no longer holds it, and it is still waiting until it reaches here.
        let starts = released + pause;
        frame(&mut turn, starts, false, false);
        turn.held(false);
        assert!(turn.waiting(starts + ms(50)));
        // Then the held reply is announced and plays, heard from its start, no longer waiting but speaking.
        let starts = starts + ms(100);
        assert!(turn.announced("m2"));
        frame(&mut turn, starts, true, true);
        assert!(!turn.waiting(starts));
        assert_eq!(turn.speaking_as(starts, false), Some(Speaking::Heard));
        // And once it has ended, it is their turn.
        let ended = starts + ms(1_000);
        frame(&mut turn, ended, false, false);
        turn.settle(ended, true);
        assert_eq!(turn.turn(), Some(ended));
    }

    #[test]
    fn a_hold_that_ends_without_playing_stops_waiting_after_the_hold() {
        let start = Instant::now();
        let mut turn = Turn::default();
        // Told it is not held, without having been held, changes nothing.
        turn.tick(start);
        turn.held(false);
        assert!(!turn.waiting(start));
        turn.held(true);
        turn.held(false);
        assert!(turn.waiting(start + HOLD - ms(1)));
        assert!(!turn.waiting(start + HOLD));
        // Nor is anything waiting after a reset.
        turn.held(true);
        turn.reset();
        assert!(!turn.waiting(start));
    }

    #[test]
    fn a_reset_forgets_the_skip_and_what_was_heard() {
        let start = Instant::now();
        let mut turn = Turn::default();
        turn.announced("m1");
        frame(&mut turn, start, true, true);
        turn.speak(start);
        frame(&mut turn, start + ms(10), true, true);
        turn.reset();
        assert!(!turn.is_skipped());
        assert!(!turn.speaking(start + ms(10)));
        turn.settle(start + ms(2_000), true);
        assert_eq!(turn.turn(), None);
        // The frame's time is kept.
        turn.played(true, true);
        assert!(turn.speaking(start + ms(10)));
    }
}
