// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The platform speaker, which plays other participants and the agent's speech through the client's [`tomori_client::playback::Player`]. See `design/live-audio.md`.
//!
//! The output stream stays open while connected. It opens and closes on its own thread, so that the frame does not wait for the device. Its callback renders the player at the device rate; [`crate::connection::Connection`] controls the player and silences it while push-to-talk records.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use tomori_client::playback::{Layout, Player};

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

    /// An output that plays as voice communication in audio `session`.
    #[cfg(target_os = "android")]
    pub fn communicating(stream: impl std::any::Any, session: i32) -> Self {
        Self {
            communication: Some(session),
            ..Self::new(stream)
        }
    }
}

/// How the speaker plays: as media, or as voice communication in the live microphone's audio session while it is open, so that the platform's echo canceller can work. See `design/spatial-audio.md`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
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
pub struct Speaking {
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
pub fn start(
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

/// Render the player at `sample_rate` into `output`, mono or interleaved stereo, or silence if another stream holds it.
pub fn render(player: &Shared, sample_rate: u32, layout: Layout, output: &mut [i16]) {
    match player.try_lock() {
        Ok(mut player) => player.render(sample_rate, layout, output),
        // A stream that is being replaced, or a poisoned player, plays nothing rather than waiting.
        Err(_) => output.fill(0),
    }
}

/// Render into `output`, `channels` interleaved samples per frame: mono for one channel, and otherwise stereo in the first two with the rest silent. `stereo` is scratch space, kept between calls.
pub fn render_channels(
    player: &Shared,
    sample_rate: u32,
    channels: usize,
    output: &mut [i16],
    stereo: &mut Vec<i16>,
) {
    match channels {
        0 => output.fill(0),
        1 => render(player, sample_rate, Layout::Mono, output),
        2 => render(player, sample_rate, Layout::Stereo, output),
        _ => {
            stereo.resize(output.len() / channels * 2, 0);
            render(player, sample_rate, Layout::Stereo, stereo);
            for (frame, pair) in output
                .chunks_exact_mut(channels)
                .zip(stereo.as_chunks::<2>().0)
            {
                frame[..2].copy_from_slice(pair);
                frame[2..].fill(0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tomori_client::playback::{MixerOptions, spatial::Listener};
    use tomori_protocol::{AudioFrame, Gate};

    fn player() -> (
        std::sync::mpsc::SyncSender<AudioFrame>,
        Shared,
        tomori_client::playback::PlayerControl,
    ) {
        let (sender, audio) = mpsc::sync_channel(8);
        let (player, control) = Player::new(audio, MixerOptions::default());
        (sender, Arc::new(Mutex::new(player)), control)
    }

    #[test]
    fn renders_mono_stereo_and_stereo_into_more_channels() {
        let (sender, player, control) = player();
        control.set_scene(|scene| {
            scene.listener = Some(Listener {
                position: [0.0, 1.6, 0.0],
                orientation: [0.0, 0.0, 0.0, 1.0],
            });
            scene.sources = vec![(1, [-2.0, 1.6, 0.0])];
        });
        let frame = |sequence| AudioFrame {
            gate: Gate::Live,
            source: 1,
            sequence,
            timestamp: 0,
            sample_rate: 48_000,
            samples: vec![1_000; 960],
        };
        for sequence in 0..5 {
            sender.send(frame(sequence)).unwrap();
        }
        let mut stereo = Vec::new();
        // Buffer to the jitter target first.
        let mut output = vec![0; 960 * 4];
        render_channels(&player, 48_000, 4, &mut output, &mut stereo);
        render_channels(&player, 48_000, 4, &mut output, &mut stereo);
        let channel = |index: usize| {
            output
                .iter()
                .skip(index)
                .step_by(4)
                .map(|s| i64::from(*s).abs())
                .sum::<i64>()
        };
        assert!(channel(0) > 2 * channel(1));
        assert_eq!(channel(2) + channel(3), 0);
        let mut mono = vec![1; 8];
        render_channels(&player, 48_000, 1, &mut mono, &mut stereo);
        let mut none: Vec<i16> = vec![1; 4];
        render_channels(&player, 48_000, 0, &mut none, &mut stereo);
        assert_eq!(none, [0; 4]);
        // Held by another stream: silence rather than waiting.
        let held = player.lock().unwrap();
        let mut output = vec![1; 4];
        render(&player, 48_000, Layout::Stereo, &mut output);
        assert_eq!(output, [0; 4]);
        drop(held);
    }
}
