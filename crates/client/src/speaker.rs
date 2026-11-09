// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The platform speaker, which plays other participants and the agent's speech through the client's [`crate::playback::Player`], behind a trait that each client's devices implement. See `design/live-audio.md`.
//!
//! The output stream stays open while connected. It opens and closes on its own thread, so that the frame does not wait for the device. Its callback renders the player at the device rate; [`crate::connection::Connection`] controls the player and silences it while push-to-talk records. `tomori-scene`'s `playback` module renders the player into a device's format.
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc::{self, RecvTimeoutError},
};
use std::time::{Duration, Instant};

use crate::playback::Player;

/// Told, from any thread, why the output stopped.
pub type Failed = Box<dyn Fn(String) + Send + Sync>;

/// The player, shared with the output callback. Only the callback of one open stream uses it at a time; the application keeps it so that a new stream can take over after a failure.
pub type Shared = Arc<Mutex<Player>>;

/// How often the speaker thread asks an output how many times it has run out of audio.
#[cfg(not(test))]
const UNDERRUN_POLL: Duration = Duration::from_secs(1);
#[cfg(test)]
const UNDERRUN_POLL: Duration = Duration::from_millis(5);

/// An open output stream, stopped when dropped. Created and dropped on the speaker thread.
pub struct Output {
    _stream: Box<dyn std::any::Any>,
    /// The audio session it plays in as voice communication, or `None` as media.
    communication: Option<i32>,
    /// How many times the device has run out of audio to play since it opened, where the platform counts it.
    underruns: Option<Box<dyn Fn() -> u32>>,
}

impl Output {
    pub fn new(stream: impl std::any::Any) -> Self {
        Self {
            _stream: Box::new(stream),
            communication: None,
            underruns: None,
        }
    }

    /// An output whose device counts the times it ran out of audio to play, as `count` says, such as when its callback
    /// was late, so that the speaker thread says when it does. See `design/live-audio.md`.
    pub fn with_underruns(self, count: impl Fn() -> u32 + 'static) -> Self {
        Self {
            underruns: Some(Box::new(count)),
            ..self
        }
    }

    /// An output that plays as voice communication in audio `session`, as Android's does for the live microphone.
    pub fn communicating(stream: impl std::any::Any, session: i32) -> Self {
        Self {
            communication: Some(session),
            ..Self::new(stream)
        }
    }
}

/// How the speaker plays: as media, or as voice communication in the live microphone's audio session while it is open, so that the platform's echo canceller can work. See `design/spatial-audio.md`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Route {
    /// The live microphone's audio session, to play in as voice communication.
    pub communication: Option<i32>,
}

/// A destination for played audio.
pub trait Playback: Send + Sync {
    /// Start playing `player` until the returned output is dropped, as `route` says where the platform can. Called on the speaker thread.
    fn open(&self, player: Shared, failed: Failed, route: Route) -> Result<Output, String>;
}

/// The speaker thread, which opens an output and keeps it until this is dropped, then closes it. Dropping does not wait for the device to close.
pub(crate) struct Speaking {
    _stop: mpsc::Sender<()>,
    opened: Arc<AtomicBool>,
    /// The session the open output plays in as voice communication, or 0.
    communication: Arc<std::sync::atomic::AtomicI32>,
}

impl Speaking {
    /// Whether the output has opened. It may have failed since.
    pub fn is_open(&self) -> bool {
        self.opened.load(Ordering::Relaxed)
    }

    /// The audio session the output opened in as voice communication, or `None` if it plays as media or has not opened: it may have fallen back to media.
    pub fn communication(&self) -> Option<i32> {
        Some(self.communication.load(Ordering::Relaxed)).filter(|session| *session != 0)
    }
}

/// Open `playback` on its own thread. Failing to start the thread or to open, and failures once open, are reported through `failed`.
pub(crate) fn start(
    playback: Arc<dyn Playback>,
    player: Shared,
    failed: Failed,
    route: Route,
) -> Speaking {
    let (stop, stopped) = mpsc::channel::<()>();
    let failed: Arc<dyn Fn(String) + Send + Sync> = Arc::from(failed);
    let opened = Arc::new(AtomicBool::new(false));
    let thread_opened = opened.clone();
    let communication = Arc::new(std::sync::atomic::AtomicI32::new(0));
    let thread_communication = communication.clone();
    let report = failed.clone();
    let started = crate::system::spawn("tomori-speaker", move || {
        let report = failed.clone();
        match playback.open(player, Box::new(move |error| report(error)), route) {
            // Kept until the speaker is dropped, which disconnects the channel.
            Ok(output) => {
                thread_communication.store(output.communication.unwrap_or(0), Ordering::Relaxed);
                thread_opened.store(true, Ordering::Relaxed);
                let mut heard = Underruns::default();
                while let Err(RecvTimeoutError::Timeout) = stopped.recv_timeout(UNDERRUN_POLL) {
                    if let Some(count) = &output.underruns {
                        heard.counted(count(), Instant::now());
                    }
                }
                drop(output);
            }
            Err(error) => failed(error),
        }
    });
    // Reported as a failure to open, so that the speaker is opened again later.
    if let Err(error) = started {
        report(format!("Cannot start the speaker thread: {error}"));
    }
    Speaking {
        _stop: stop,
        opened,
        communication,
    }
}

/// How often running out of audio is said, at most, as recurring events are (see `design/client-logging.md`).
const UNDERRUN_REPORT: Duration = Duration::from_secs(30);

/// The times an output has run out of audio to play: as last counted, when that was last said, and how many times
/// since.
#[derive(Debug, Default)]
struct Underruns {
    counted: u32,
    said: Option<Instant>,
    unsaid: u32,
}

impl Underruns {
    /// The output counts `count` at `now`: say how many more times it ran out of audio since this was last said, if it
    /// did, at most once every [`UNDERRUN_REPORT`]. Returns what it said.
    fn counted(&mut self, count: u32, now: Instant) -> Option<String> {
        // A count that went back, as when a platform starts counting again, is counted on from there.
        self.unsaid += count.saturating_sub(self.counted);
        self.counted = count;
        if self.unsaid == 0
            || self
                .said
                .is_some_and(|said| now.saturating_duration_since(said) < UNDERRUN_REPORT)
        {
            return None;
        }
        let said = format!(
            "The speaker ran out of audio to play {}, so what played was heard with a gap: this device's playback fell behind",
            match self.unsaid {
                1 => "once".to_owned(),
                more => format!("{more} times"),
            }
        );
        self.said = Some(now);
        self.unsaid = 0;
        log::info!("{said}.");
        Some(said)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, AtomicUsize};

    #[test]
    fn running_out_of_audio_is_said_as_it_is_counted_at_most_every_thirty_seconds() {
        let now = Instant::now();
        let mut underruns = Underruns::default();
        assert_eq!(underruns.counted(0, now), None);
        assert_eq!(
            underruns.counted(1, now).as_deref(),
            Some(
                "The speaker ran out of audio to play once, so what played was heard with a gap: this device's playback fell behind"
            )
        );
        // Counted again within thirty seconds, it waits to be said with the next.
        assert_eq!(underruns.counted(4, now + UNDERRUN_REPORT / 2), None);
        // A count that went back, as when a platform starts counting again, is counted on from there.
        assert_eq!(underruns.counted(2, now + UNDERRUN_REPORT / 2), None);
        // Thirty seconds on, the next is said with those.
        assert!(
            underruns
                .counted(3, now + UNDERRUN_REPORT)
                .unwrap()
                .contains("to play 4 times,")
        );
        assert_eq!(underruns.counted(3, now + UNDERRUN_REPORT * 2), None);
    }

    /// Plays nowhere, counting its underruns as `underruns` holds them and how often it was asked in `asked`.
    struct Counting {
        underruns: Arc<AtomicU32>,
        asked: Arc<AtomicUsize>,
    }

    impl Playback for Counting {
        fn open(&self, _: Shared, _: Failed, _: Route) -> Result<Output, String> {
            let (underruns, asked) = (self.underruns.clone(), self.asked.clone());
            Ok(Output::new(()).with_underruns(move || {
                asked.fetch_add(1, Ordering::Relaxed);
                underruns.load(Ordering::Relaxed)
            }))
        }
    }

    #[test]
    fn the_speaker_thread_asks_an_open_output_how_often_it_ran_out_of_audio() {
        let (_, audio) = mpsc::sync_channel(1);
        let (player, _) = crate::playback::Player::new(audio, Default::default());
        let underruns = Arc::new(AtomicU32::new(0));
        let asked = Arc::new(AtomicUsize::new(0));
        let speaking = start(
            Arc::new(Counting {
                underruns: underruns.clone(),
                asked: asked.clone(),
            }),
            Arc::new(Mutex::new(player)),
            Box::new(|_| {}),
            Route::default(),
        );
        let waited = |at_least: usize| {
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            while asked.load(Ordering::Relaxed) < at_least {
                assert!(std::time::Instant::now() < deadline, "never asked");
                std::thread::sleep(UNDERRUN_POLL);
            }
        };
        waited(1);
        underruns.store(2, Ordering::Relaxed);
        let now = asked.load(Ordering::Relaxed);
        waited(now + 2);
        assert!(speaking.is_open());
        drop(speaking);
    }
}
