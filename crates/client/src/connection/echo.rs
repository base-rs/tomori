// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The echo test: how much of the speaker the live microphone hears, measured once as it opens. See `design/spatial-audio.md`.
use std::time::{Duration, Instant};

/// The energy of captured samples, from −1 to 1, and how many there were.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct Heard {
    pub(super) energy: f64,
    pub(super) samples: u64,
}

impl Heard {
    pub(super) fn add(&mut self, other: Heard) {
        self.energy += other.energy;
        self.samples += other.samples;
    }

    /// The level heard, in dBFS RMS, if anything was.
    pub(super) fn level(&self) -> Option<f64> {
        (self.samples > 0).then(|| 10.0 * (self.energy / self.samples as f64).max(1e-12).log10())
    }
}

/// When the echo test listens, from when the live microphone opens: first the room, then the probe the speaker plays. See `design/spatial-audio.md`.
pub(super) const ECHO_FLOOR: std::ops::Range<Duration> =
    Duration::from_millis(300)..Duration::from_millis(1_000);
pub(super) const ECHO_PROBE: Duration = Duration::from_millis(1_000);
/// How long the echo test waits for the speaker to play as it needs before giving up, saying so.
pub(super) const ECHO_WAIT: Duration = Duration::from_secs(5);
/// The part of the probe measured, after it starts: its steady middle, allowing for the speaker's and microphone's latency.
pub(super) const ECHO_HEARD: std::ops::Range<Duration> =
    Duration::from_millis(300)..Duration::from_millis(1_300);

/// The echo test, for the live microphone opened at `opened`: what it heard of the room and of the probe. See `design/spatial-audio.md`.
#[derive(Debug)]
pub(super) struct EchoTest {
    /// How the live microphone captures, for the log: with the platform's echo canceller or without.
    pub(super) label: &'static str,
    /// Whether the speaker plays as voice communication in the microphone's session for this test, so that it waits for that.
    pub(super) routed: bool,
    /// When the live microphone opened, while the test waits for the speaker to play as it should.
    pub(super) armed: Option<Instant>,
    /// The audio session the speaker played in as voice communication during the test, or `None` as media, for the log.
    pub(super) speaker: Option<i32>,
    pub(super) opened: Option<Instant>,
    pub(super) probed: Option<Instant>,
    pub(super) floor: Heard,
    pub(super) echo: Heard,
}

impl EchoTest {
    /// Start timing once the speaker plays as the test needs, after any reopening for the live microphone: as voice communication, not having fallen back to media, when the test needs that. `playing` is how the open speaker plays at `now`, `None` if it is not open, and `Some(None)` as media. Gives up, saying so, after [`ECHO_WAIT`].
    pub(super) fn arm(&mut self, playing: Option<Option<i32>>, now: Instant) {
        let Some(armed) = self.armed else {
            return;
        };
        match playing {
            Some(communication) if !self.routed || communication.is_some() => {
                self.armed = None;
                self.speaker = communication;
                self.opened = Some(now);
            }
            _ if now.saturating_duration_since(armed) >= ECHO_WAIT => {
                self.armed = None;
                log::warn!(
                    "Echo test ({}): nothing measured, as the speaker did not play {} within {} seconds",
                    self.label,
                    if self.routed {
                        "as voice communication in the live microphone's session"
                    } else {
                        "at all"
                    },
                    ECHO_WAIT.as_secs()
                );
            }
            _ => {}
        }
    }

    /// Attribute what the microphone `heard`, up to `now`, to the room or the probe. Returns the result to log once both are measured, and whether to play the probe now.
    pub(super) fn update(&mut self, heard: Heard, now: Instant) -> (Option<String>, bool) {
        let Some(opened) = self.opened else {
            return (None, false);
        };
        let since = now.saturating_duration_since(opened);
        let mut probe = false;
        match self.probed {
            None if since >= ECHO_PROBE => {
                self.probed = Some(now);
                probe = true;
            }
            None if ECHO_FLOOR.contains(&since) => self.floor.add(heard),
            None => {}
            Some(probed) => {
                let since = now.saturating_duration_since(probed);
                if ECHO_HEARD.contains(&since) {
                    self.echo.add(heard);
                } else if since >= ECHO_HEARD.end {
                    self.opened = None;
                    let level = |heard: Heard| {
                        heard
                            .level()
                            .map_or_else(|| "nothing".into(), |level| format!("{level:.1} dBFS"))
                    };
                    let probe = 20.0 * f64::from(crate::playback::PROBE_LEVEL).log10();
                    let below = self.echo.level().map_or_else(String::new, |echo| {
                        format!(", {:.1} dB below the probe", probe - echo)
                    });
                    // Within a few dB of the room, the echo is lost in it, and the measurement says only that it is at most this.
                    let room = match (self.echo.level(), self.floor.level()) {
                        (Some(echo), Some(floor)) if echo - floor < 3.0 => {
                            format!(
                                ", within {:.1} dB of the room, so at most this",
                                echo - floor
                            )
                        }
                        (Some(echo), Some(floor)) => {
                            format!(", {:.1} dB above the room", echo - floor)
                        }
                        _ => String::new(),
                    };
                    return (
                        Some(format!(
                            "Echo test ({}, the speaker {}): the speaker played a probe at {probe:.1} dBFS; the microphone heard {} during it{below}{room}; the room was {}",
                            self.label,
                            self.speaker.map_or_else(
                                || "as media".into(),
                                |session| format!("as voice communication in session {session}")
                            ),
                            level(self.echo),
                            level(self.floor),
                        )),
                        false,
                    );
                }
            }
        }
        (None, probe)
    }
}

#[cfg(test)]
mod tests;
