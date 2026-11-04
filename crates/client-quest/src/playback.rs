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
    #[cfg_attr(not(target_os = "android"), allow(dead_code))]
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

/// Open `playback` on its own thread. Failing to open, and failures once open, are reported through `failed`.
pub fn start(
    playback: Arc<dyn Playback>,
    player: Shared,
    failed: Failed,
    route: Route,
) -> Result<Speaking, String> {
    let (stop, stopped) = mpsc::channel::<()>();
    let failed: Arc<dyn Fn(String) + Send + Sync> = Arc::from(failed);
    let opened = Arc::new(AtomicBool::new(false));
    let thread_opened = opened.clone();
    let communication = Arc::new(std::sync::atomic::AtomicI32::new(0));
    let thread_communication = communication.clone();
    std::thread::Builder::new()
        .name("tomori-speaker".into())
        .spawn(move || {
            let report = failed.clone();
            match playback.open(player, Box::new(move |error| report(error)), route) {
                // Kept until the speaker is dropped, which disconnects the channel.
                Ok(output) => {
                    thread_communication
                        .store(output.communication.unwrap_or(0), Ordering::Relaxed);
                    thread_opened.store(true, Ordering::Relaxed);
                    let _ = stopped.recv();
                    drop(output);
                }
                Err(error) => failed(error),
            }
        })
        .map_err(|error| format!("Cannot start the speaker thread: {error}"))?;
    Ok(Speaking {
        _stop: stop,
        opened,
        communication,
    })
}

/// Render the player at `sample_rate` into `output`, mono or interleaved stereo, or silence if another stream holds it.
fn render(player: &Shared, sample_rate: u32, layout: Layout, output: &mut [i16]) {
    match player.try_lock() {
        Ok(mut player) => player.render(sample_rate, layout, output),
        // A stream that is being replaced, or a poisoned player, plays nothing rather than waiting.
        Err(_) => output.fill(0),
    }
}

/// Render into `output`, `channels` interleaved samples per frame: mono for one channel, and otherwise stereo in the first two with the rest silent. `stereo` is scratch space, kept between calls.
fn render_channels(
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

/// Renders the player into a desktop output stream's samples, in its own format, `channels` interleaved samples per frame. Its buffers are kept between callbacks.
#[cfg(not(target_os = "android"))]
pub struct Render {
    player: Shared,
    sample_rate: u32,
    channels: usize,
    samples: Vec<i16>,
    stereo: Vec<i16>,
}

#[cfg(not(target_os = "android"))]
impl Render {
    pub fn new(player: Shared, sample_rate: u32, channels: u16) -> Self {
        Self {
            player,
            sample_rate,
            channels: usize::from(channels).max(1),
            samples: Vec::new(),
            stereo: Vec::new(),
        }
    }

    /// Fill `data` with what the player plays next.
    pub fn fill<T: cpal::Sample + cpal::FromSample<i16>>(&mut self, data: &mut [T]) {
        self.samples.resize(data.len(), 0);
        render_channels(
            &self.player,
            self.sample_rate,
            self.channels,
            &mut self.samples,
            &mut self.stereo,
        );
        for (output, &sample) in data.iter_mut().zip(&self.samples) {
            *output = T::from_sample(sample);
        }
    }
}

/// Handle a desktop output stream's `error`. The stream keeps playing after most: a glitch in real-time audio is not worth reopening for, and underruns can come in bursts whenever the system is busy. Anything else is `failed`.
#[cfg(not(target_os = "android"))]
pub fn output_failed(error: &cpal::Error, failed: &Failed) {
    match error.kind() {
        cpal::ErrorKind::DeviceChanged | cpal::ErrorKind::RealtimeDenied => {
            log::info!("Speaker: {error}");
        }
        cpal::ErrorKind::Xrun => log::debug!("Speaker: {error}"),
        _ => failed(format!("The speaker failed: {error}")),
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

    #[cfg(not(target_os = "android"))]
    #[test]
    fn a_desktop_stream_is_filled_in_its_own_format() {
        let (_sender, player, _control) = player();
        // Nothing to play: silence, in each format's own terms.
        let mut floats = [1.0_f32; 4];
        Render::new(player.clone(), 48_000, 2).fill(&mut floats);
        assert_eq!(floats, [0.0; 4]);
        let mut unsigned = [0_u16; 3];
        Render::new(player, 48_000, 0).fill(&mut unsigned);
        assert_eq!(unsigned, [32_768; 3]);
    }

    #[cfg(not(target_os = "android"))]
    #[test]
    fn only_desktop_output_errors_that_stop_it_are_reported() {
        let reported = Arc::new(Mutex::new(Vec::new()));
        let sink = reported.clone();
        let failed: Failed = Box::new(move |error| sink.lock().unwrap().push(error));
        for kind in [
            cpal::ErrorKind::DeviceChanged,
            cpal::ErrorKind::RealtimeDenied,
            cpal::ErrorKind::Xrun,
        ] {
            output_failed(&cpal::Error::new(kind), &failed);
        }
        assert!(reported.lock().unwrap().is_empty());
        output_failed(
            &cpal::Error::new(cpal::ErrorKind::DeviceNotAvailable),
            &failed,
        );
        let reported = reported.lock().unwrap();
        assert_eq!(reported.len(), 1);
        assert!(reported[0].starts_with("The speaker failed: "));
    }
}

#[cfg(target_os = "android")]
pub use android::Android;

/// AAudio, at the device's own rate, with a low-latency data callback.
#[cfg(target_os = "android")]
mod android {
    use super::{Failed, Output, Playback, Route, Shared, render_channels};
    use ndk::audio::{
        AudioCallbackResult, AudioContentType, AudioDirection, AudioFormat, AudioPerformanceMode,
        AudioSharingMode, AudioStreamBuilder, AudioUsage, SessionId,
    };
    use std::num::NonZeroI32;

    pub struct Android;

    impl Playback for Android {
        fn open(&self, player: Shared, failed: Failed, route: Route) -> Result<Output, String> {
            let failed: std::sync::Arc<dyn Fn(String) + Send + Sync> = std::sync::Arc::from(failed);
            match route.communication {
                Some(session) => open(player.clone(), failed.clone(), Some(session)).or_else(|error| {
                    log::warn!("Cannot play as voice communication in session {session}, so playing as media: {error}");
                    open(player, failed, None)
                }),
                None => open(player, failed, None),
            }
        }
    }

    /// Open the speaker, as voice communication in `session` when given, or else as media.
    fn open(
        player: Shared,
        failed: std::sync::Arc<dyn Fn(String) + Send + Sync>,
        session: Option<i32>,
    ) -> Result<Output, String> {
        // Stereo 16-bit at the device's rate, rendered straight into AAudio's buffer, so that each source is heard from where it is; see `design/spatial-audio.md`.
        let mut stereo = Vec::new();
        let player_for_prepare = player.clone();
        let builder = AudioStreamBuilder::new()
            .map_err(|error| format!("Cannot open the speaker: {error}"))?
            .direction(AudioDirection::Output)
            .sharing_mode(AudioSharingMode::Shared)
            .performance_mode(AudioPerformanceMode::LowLatency)
            .content_type(AudioContentType::Speech)
            .format(AudioFormat::PCM_I16)
            .channel_count(2);
        let builder = match session.and_then(NonZeroI32::new) {
            Some(session) => builder
                .usage(AudioUsage::VoiceCommunication)
                .session_id(Some(SessionId::Allocated(session))),
            None => builder.usage(AudioUsage::Media),
        };
        let stream = builder
            .data_callback(Box::new(move |stream, data, frames| {
                let channels = stream.channel_count().max(1) as usize;
                let frames = frames.max(0) as usize;
                let sample_rate = stream.sample_rate().max(0) as u32;
                // SAFETY: AAudio hands the callback a buffer of `frames` frames of the stream's format, which was checked to be 16-bit when the stream opened, with `channels` samples each.
                let output = unsafe {
                    std::slice::from_raw_parts_mut(data.cast::<i16>(), frames * channels)
                };
                render_channels(&player, sample_rate, channels, output, &mut stereo);
                AudioCallbackResult::Continue
            }))
            .error_callback(Box::new(move |_, error| {
                failed(format!("The speaker failed: {error}"))
            }))
            .open_stream()
            .map_err(|error| format!("Cannot open the speaker: {error}"))?;
        if stream.format() != AudioFormat::PCM_I16 {
            return Err("The speaker opened in an unexpected format.".into());
        }
        // The cues are synthesized for the device's rate here, before the stream starts, rather than in its first callback.
        if let Ok(mut player) = player_for_prepare.lock() {
            player.prepare(stream.sample_rate().max(0) as u32);
        }
        stream
            .request_start()
            .map_err(|error| format!("Cannot start the speaker: {error}"))?;
        log::info!(
            "Speaker opened at {} Hz, {} channels, as {}",
            stream.sample_rate(),
            stream.channel_count(),
            match session {
                Some(session) => format!("voice communication in session {session}"),
                None => "media".into(),
            }
        );
        Ok(match session {
            Some(session) => Output::communicating(Stream(stream), session),
            None => Output::new(Stream(stream)),
        })
    }

    /// Stops the stream before closing it.
    struct Stream(ndk::audio::AudioStream);

    impl Drop for Stream {
        fn drop(&mut self) {
            let _ = self.0.request_stop();
        }
    }
}
