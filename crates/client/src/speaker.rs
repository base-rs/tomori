// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The platform speaker, which plays other participants and the agent's speech through the client's [`crate::playback::Player`], behind a trait that each client's devices implement. See `design/live-audio.md`.
//!
//! The output stream stays open while connected. It opens and closes on its own thread, so that the frame does not wait for the device. Its callback renders the player at the device rate; [`crate::connection::Connection`] controls the player and silences it while push-to-talk records. `tomori-scene`'s `playback` module renders the player into a device's format.
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc,
};

use crate::playback::Player;

/// Told, from any thread, why the output stopped.
pub type Failed = Box<dyn Fn(String) + Send + Sync>;

/// The player, shared with the output callback. Only the callback of one open stream uses it at a time; the application keeps it so that a new stream can take over after a failure.
pub type Shared = Arc<Mutex<Player>>;

/// An open output stream, stopped when dropped. Created and dropped on the speaker thread.
pub struct Output {
    _stream: Box<dyn std::any::Any>,
    /// The audio session it plays in as voice communication, or `None` as media.
    communication: Option<i32>,
}

impl Output {
    pub fn new(stream: impl std::any::Any) -> Self {
        Self {
            _stream: Box::new(stream),
            communication: None,
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
                let _ = stopped.recv();
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
