// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The platform microphone, opened for live audio or push-to-talk. See `design/live-audio.md`.
//!
//! Capture hands mono samples to a callback on its own thread; [`crate::connection::Connection`] passes them to the client's [`client::Microphone`], which frames and sends them.

/// Receives captured mono samples in the range -1 to 1, on the capture thread.
pub type Samples = Box<dyn FnMut(&[f32]) + Send>;

/// Told, from any thread, why capture stopped early.
pub type Failed = Box<dyn Fn(String) + Send + Sync>;

/// Whether the connection on which live capture started is still usable.
pub type Active = Box<dyn Fn() -> bool + Send>;

/// Capture in progress, stopped when dropped. Samples already delivered are not lost.
pub struct Recording {
    pub sample_rate: u32,
    _stream: Box<dyn std::any::Any>,
}

impl Recording {
    pub fn new(sample_rate: u32, stream: impl std::any::Any) -> Self {
        Self {
            sample_rate,
            _stream: Box::new(stream),
        }
    }
}

/// A source of microphone audio.
pub trait Capture {
    /// Whether the app may record, asking the participant if it has not yet. A request is answered later, so `Ok(false)` means ask again after they respond.
    fn permit(&mut self) -> Result<bool, String> {
        Ok(true)
    }

    /// Ask for permission ahead of the first recording, without using up the
    /// request [`Self::permit`] makes, so that a dismissed request is asked
    /// again when the participant first uses the microphone.
    fn request(&mut self) -> Result<bool, String> {
        Ok(true)
    }

    /// Start capturing at a rate between 8 and 48 kHz, which the protocol carries.
    fn open(&mut self, samples: Samples, failed: Failed) -> Result<Recording, String>;

    /// Capture for an open microphone, stopping when its connection is lost.
    /// Platforms must opt in with a capture implementation that can stop on
    /// the capture thread, even if the application thread is stalled.
    fn open_live(
        &mut self,
        _samples: Samples,
        _failed: Failed,
        _active: Active,
    ) -> Result<Recording, String> {
        Err("Live microphone is not available on this platform.".into())
    }

    /// The audio session the open live microphone captures in, where the platform allocates one, so that the speaker can play in the same session while it is open. See `design/spatial-audio.md`.
    fn live_session(&self) -> Option<i32> {
        None
    }
}

/// Average interleaved channels into mono.
#[cfg_attr(target_os = "android", allow(dead_code))]
fn downmix(samples: &[f32], channels: usize, output: &mut Vec<f32>) {
    output.clear();
    if channels <= 1 {
        output.extend_from_slice(samples);
        return;
    }
    output.extend(
        samples
            .chunks_exact(channels)
            .map(|frame| frame.iter().sum::<f32>() / channels as f32),
    );
}

/// A pull-based input stream, such as AAudio's. Separated from the device so
/// that the capture boundary can be tested deterministically.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
trait InputStream {
    fn start(&mut self) -> Result<(), String>;
    /// Read up to `buffer.len()` samples, waiting at most `timeout_ns`; zero
    /// returns only what the device has already captured.
    fn read(&mut self, buffer: &mut [f32], timeout_ns: i64) -> Result<usize, String>;
    fn stop(&mut self);
    /// How many times the device has discarded audio because it was not read
    /// in time. A device that cannot tell reports a constant, such as zero or
    /// an error code, which is never taken as a loss.
    fn overruns(&self) -> i32 {
        0
    }
}

#[cfg_attr(not(target_os = "android"), allow(dead_code))]
const CONNECTION_LOST: &str = "Connection lost; live microphone stopped.";

/// How long one read may wait for audio, so that stop and disconnection are
/// noticed promptly.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
const READ_TIMEOUT_NS: i64 = 100_000_000;

/// The most non-blocking reads made to collect what the device had already
/// captured when capture was stopped; a well-behaved device runs dry first.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
const MAX_DRAIN_READS: usize = 64;

/// The shortest interval between warnings that the live microphone lost audio.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
const OVERRUN_LOG: std::time::Duration = std::time::Duration::from_secs(30);

/// Capture until `stop` is set or, for live audio, `active` reports that the
/// connection ended.
///
/// The capture boundary is the moment `stop` is observed. Every sample the
/// device captured before it is delivered: a read in flight when stop is set,
/// then what the device already holds, read without waiting, including a final
/// partial frame. If that cannot be read intact, including when the device
/// reports an overrun during push-to-talk, capture fails, so the utterance is
/// cancelled visibly instead of being sent with a gap. Live audio tolerates an
/// overrun, which only glitches what the other participants hear.
///
/// A connection that ended before or while the device opened never starts
/// the stream, and no sample is delivered once `active` reports false. A
/// disconnection racing the start itself stops the stream before any read.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
fn run_capture<S: InputStream>(
    open: impl FnOnce() -> Result<S, String>,
    samples: &mut dyn FnMut(&[f32]),
    stop: &std::sync::atomic::AtomicBool,
    active: Option<&Active>,
    buffer: &mut [f32],
) -> Result<(), String> {
    use crate::log_cadence::Cadence;
    use std::sync::atomic::Ordering;
    let current = || active.is_none_or(|active| active());
    if !current() {
        return Err(CONNECTION_LOST.into());
    }
    let mut stream = open()?;
    // Opening the device can take a while; the connection may have ended.
    if !current() {
        return Err(CONNECTION_LOST.into());
    }
    if stop.load(Ordering::Acquire) {
        // Released before capture began: nothing was captured.
        return Ok(());
    }
    stream.start()?;
    let overruns = std::cell::Cell::new(stream.overruns());
    // Overruns that keep recurring would otherwise warn on every read; see `design/client-logging.md`.
    let unreported = std::cell::RefCell::new((Cadence::new(OVERRUN_LOG), 0_u64));
    // Push-to-talk (no `active`) must reach recognition intact.
    let intact = |stream: &S| -> Result<(), String> {
        let count = stream.overruns();
        let lost = count.saturating_sub(overruns.get());
        if lost > 0 {
            if active.is_none() {
                return Err("Microphone lost audio; not sent.".into());
            }
            overruns.set(count);
            let (cadence, pending) = &mut *unreported.borrow_mut();
            *pending += u64::from(lost.unsigned_abs());
            if cadence.record(std::time::Instant::now()).is_some() {
                warn_overruns(std::mem::take(pending));
            }
        }
        Ok(())
    };
    let result = loop {
        if !current() {
            break Err(CONNECTION_LOST.into());
        }
        if stop.load(Ordering::Acquire) {
            break drain(&mut stream, samples, &current, &intact, buffer);
        }
        match stream.read(buffer, READ_TIMEOUT_NS) {
            // A successful read was captured before stop was observed, so it
            // is delivered even if stop was set while it was in flight.
            Ok(count) => {
                if let Err(error) = intact(&stream) {
                    break Err(error);
                }
                if current() {
                    samples(&buffer[..count]);
                }
            }
            Err(error) => break Err(error),
        }
    };
    stream.stop();
    let (_, pending) = unreported.into_inner();
    if pending > 0 {
        warn_overruns(pending);
    }
    result
}

#[cfg_attr(not(target_os = "android"), allow(dead_code))]
fn warn_overruns(lost: u64) {
    log::warn!("The live microphone lost audio ({lost} overruns).");
}

/// Deliver what the device captured before the boundary, without waiting.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
fn drain<S: InputStream>(
    stream: &mut S,
    samples: &mut dyn FnMut(&[f32]),
    current: &dyn Fn() -> bool,
    intact: &dyn Fn(&S) -> Result<(), String>,
    buffer: &mut [f32],
) -> Result<(), String> {
    for _ in 0..MAX_DRAIN_READS {
        let count = stream.read(buffer, 0)?;
        intact(stream)?;
        if count == 0 {
            return Ok(());
        }
        if !current() {
            return Err(CONNECTION_LOST.into());
        }
        samples(&buffer[..count]);
    }
    Err("The microphone kept delivering audio after it was stopped.".into())
}

#[cfg(not(target_os = "android"))]
pub use desktop::Desktop;

/// The default input device, through cpal.
#[cfg(not(target_os = "android"))]
mod desktop {
    use super::{Capture, Failed, Recording, Samples, downmix};
    use cpal::{
        ErrorKind, FromSample, SampleFormat, SizedSample, StreamConfig,
        traits::{DeviceTrait, HostTrait, StreamTrait},
    };

    pub struct Desktop;

    /// The protocol's range of sample rates.
    const RATES: std::ops::RangeInclusive<u32> = 8_000..=48_000;

    impl Capture for Desktop {
        fn open(&mut self, samples: Samples, failed: Failed) -> Result<Recording, String> {
            let started = std::time::Instant::now();
            let device = cpal::default_host()
                .default_input_device()
                .ok_or("No microphone is available.")?;
            let supported = device
                .default_input_config()
                .map_err(|error| format!("Cannot use the microphone: {error}"))?;
            let mut config: StreamConfig = supported.config();
            if !RATES.contains(&config.sample_rate) {
                // Ask for the nearest rate the protocol carries; the device refuses it if it cannot.
                config.sample_rate = config.sample_rate.clamp(*RATES.start(), *RATES.end());
            }
            let stream = match supported.sample_format() {
                SampleFormat::F32 => build::<f32>(&device, &config, samples, failed),
                SampleFormat::I16 => build::<i16>(&device, &config, samples, failed),
                SampleFormat::I32 => build::<i32>(&device, &config, samples, failed),
                SampleFormat::U16 => build::<u16>(&device, &config, samples, failed),
                format => {
                    return Err(format!(
                        "The microphone's {format} samples are not supported."
                    ));
                }
            }?;
            stream
                .play()
                .map_err(|error| format!("Cannot start the microphone: {error}"))?;
            log::info!(
                "Microphone opened at {} Hz, {} channels, in {:?}",
                config.sample_rate,
                config.channels,
                started.elapsed()
            );
            Ok(Recording::new(config.sample_rate, stream))
        }
    }

    fn build<T>(
        device: &cpal::Device,
        config: &StreamConfig,
        mut samples: Samples,
        failed: Failed,
    ) -> Result<cpal::Stream, String>
    where
        T: SizedSample,
        f32: FromSample<T>,
    {
        let channels = usize::from(config.channels);
        let (mut converted, mut mono) = (Vec::new(), Vec::new());
        device
            .build_input_stream::<T, _, _>(
                *config,
                move |data: &[T], _| {
                    converted.clear();
                    converted.extend(data.iter().map(|sample| sample.to_sample::<f32>()));
                    downmix(&converted, channels, &mut mono);
                    samples(&mono);
                },
                move |error: cpal::Error| match error.kind() {
                    // The stream keeps capturing without a gap.
                    ErrorKind::DeviceChanged | ErrorKind::RealtimeDenied => {
                        log::info!("Microphone: {error}");
                    }
                    // Anything else, including an overrun, loses audio, which must not be sent as if whole.
                    _ => failed(format!("The microphone failed: {error}")),
                },
                None,
            )
            .map_err(|error| format!("Cannot open the microphone: {error}"))
    }
}

#[cfg(target_os = "android")]
pub use android::Android;

/// AAudio, at the 16 kHz the recognizer uses, after asking for `RECORD_AUDIO`.
#[cfg(target_os = "android")]
mod android {
    use super::{Active, Capture, Failed, InputStream, Recording, Samples, run_capture};
    use ndk::audio::{
        AudioDirection, AudioFormat, AudioInputPreset, AudioSharingMode, AudioStream,
        AudioStreamBuilder, SessionId,
    };
    use std::{
        num::NonZeroI32,
        sync::{
            Arc,
            atomic::{AtomicBool, AtomicI32, Ordering},
        },
        thread::JoinHandle,
    };

    const SAMPLE_RATE: u32 = 16_000;

    pub struct Android {
        android: Arc<crate::android::Android>,
        requested: bool,
        requested_early: bool,
        /// The live microphone's audio session while it is open, or 0.
        session: Arc<AtomicI32>,
        /// The audio session the live microphone last used, or 0, used again each time it opens so that the speaker, playing in it, need not reopen; as when push-to-talk during live audio ends.
        kept: Arc<AtomicI32>,
        /// Capture live audio with the recognition preset and no echo canceller, so that the echo test can compare; see `design/spatial-audio.md`.
        uncancelled: bool,
        /// Enable the echo canceller on the live microphone's session, or disable it, for the echo test to compare.
        canceller: bool,
    }

    impl Android {
        pub fn new(
            android: Arc<crate::android::Android>,
            uncancelled: bool,
            canceller: bool,
        ) -> Self {
            Self {
                android,
                requested: false,
                requested_early: false,
                session: Arc::default(),
                kept: Arc::default(),
                uncancelled,
                canceller,
            }
        }
    }

    impl Capture for Android {
        /// Asks once per launch; after that, the participant must allow it in Settings, since Android may not ask again.
        fn permit(&mut self) -> Result<bool, String> {
            let asked = self.requested;
            let granted = self
                .android
                .check_or_request("android.permission.RECORD_AUDIO", &mut self.requested)
                .map_err(|error| format!("Cannot ask for the microphone: {error:#}"))?;
            if !granted && asked {
                return Err("Allow the microphone for Tomori in Settings.".into());
            }
            Ok(granted)
        }

        fn request(&mut self) -> Result<bool, String> {
            self.android
                .check_or_request("android.permission.RECORD_AUDIO", &mut self.requested_early)
                .map_err(|error| format!("Cannot ask for the microphone: {error:#}"))
        }

        /// The stream opens on the capture thread, so that the frame does not wait for it; failing to open is reported through `failed`.
        fn open(&mut self, samples: Samples, failed: Failed) -> Result<Recording, String> {
            self.start(samples, failed, AudioInputPreset::VoiceRecognition, None)
        }

        fn open_live(
            &mut self,
            samples: Samples,
            failed: Failed,
            active: Active,
        ) -> Result<Recording, String> {
            let preset = if self.uncancelled {
                AudioInputPreset::VoiceRecognition
            } else {
                AudioInputPreset::VoiceCommunication
            };
            self.start(samples, failed, preset, Some(active))
        }

        fn live_session(&self) -> Option<i32> {
            Some(self.session.load(Ordering::Relaxed)).filter(|session| *session != 0)
        }
    }

    impl Android {
        fn start(
            &mut self,
            samples: Samples,
            failed: Failed,
            preset: AudioInputPreset,
            active: Option<Active>,
        ) -> Result<Recording, String> {
            let stop = Arc::new(AtomicBool::new(false));
            let thread_stop = stop.clone();
            // Live audio with the communication preset captures in an audio session of its own, with the platform's echo canceller attached where it has one.
            // Live audio without a session, as the echo test's `uncancelled` run captures, still says what shapes it, on the capture thread.
            let unsessioned = (active.is_some() && preset != AudioInputPreset::VoiceCommunication)
                .then(|| self.android.clone());
            let session = (active.is_some() && preset == AudioInputPreset::VoiceCommunication)
                .then(|| Session {
                    current: self.session.clone(),
                    kept: self.kept.clone(),
                    android: self.android.clone(),
                    canceller: self.canceller,
                });
            let thread = std::thread::Builder::new()
                .name("tomori-microphone".into())
                .spawn(move || {
                    if let Some(android) = unsessioned {
                        android.log_audio_state(None, log::Level::Info);
                    }
                    if let Err(error) = capture(samples, &thread_stop, preset, active, session) {
                        failed(error);
                    }
                })
                .map_err(|error| format!("Cannot start the microphone thread: {error}"))?;
            Ok(Recording::new(
                SAMPLE_RATE,
                Worker {
                    stop,
                    thread: Some(thread),
                },
            ))
        }
    }

    /// Stops and joins the capture thread when dropped, so that every captured sample is delivered first.
    struct Worker {
        stop: Arc<AtomicBool>,
        thread: Option<JoinHandle<()>>,
    }

    impl Drop for Worker {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Release);
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }

    fn capture(
        mut samples: Samples,
        stop: &AtomicBool,
        preset: AudioInputPreset,
        active: Option<Active>,
        session: Option<Session>,
    ) -> Result<(), String> {
        // Shared mode supports Android's resampler, so ask for the recognizer's 16 kHz mono directly.
        let builder = |session: Option<Option<SessionId>>| {
            let builder = AudioStreamBuilder::new()
                .map_err(|error| format!("Cannot open the microphone: {error}"))?
                .direction(AudioDirection::Input)
                .sharing_mode(AudioSharingMode::Shared)
                .channel_count(1)
                .sample_rate(SAMPLE_RATE as i32)
                .format(AudioFormat::PCM_Float)
                .input_preset(preset);
            // `Some(None)` allocates a new session.
            Ok::<_, String>(match session {
                Some(session) => builder.session_id(session),
                None => builder,
            })
        };
        let open = || {
            let kept = session
                .as_ref()
                .and_then(|session| NonZeroI32::new(session.kept.load(Ordering::Relaxed)));
            let opened = |session| {
                builder(session)?
                    .open_stream()
                    .map_err(|error| format!("Cannot open the microphone: {error}"))
            };
            let stream = match (&session, kept) {
                (None, _) => opened(None),
                (Some(_), None) => opened(Some(None)),
                // The session used before, or a new one if it is refused.
                (Some(_), Some(kept)) => {
                    opened(Some(Some(SessionId::Allocated(kept)))).or_else(|error| {
                        log::info!("{error} in audio session {kept} again, so allocating another");
                        opened(Some(None))
                    })
                }
            }?;
            if stream.sample_rate() != SAMPLE_RATE as i32
                || stream.channel_count() != 1
                || stream.format() != AudioFormat::PCM_Float
            {
                return Err("The microphone opened in an unexpected format.".into());
            }
            let mut canceller = None;
            if let Some(session) = &session {
                match stream.session_id() {
                    SessionId::Allocated(id) => {
                        // In full the first time in a session; again, as when push-to-talk during live audio ends, only at debug.
                        let level = if session.kept.load(Ordering::Relaxed) == id.get() {
                            log::Level::Debug
                        } else {
                            log::Level::Info
                        };
                        log::log!(
                            level,
                            "Live microphone in audio session {}, preset {preset:?}",
                            id.get()
                        );
                        // The platform attaches its echo canceller to this preset where it has one; ask for it by session, enable it (or disable it, for the echo test), and say what it and the other effects report. See `design/client-logging.md`.
                        canceller = session
                            .android
                            .echo_canceller(id.get(), session.canceller, level)
                            .unwrap_or_else(|error| {
                                log::warn!("Cannot reach the echo canceller: {error:#}");
                                None
                            });
                        session.android.log_audio_state(Some(id.get()), level);
                        session.kept.store(id.get(), Ordering::Relaxed);
                        session.current.store(id.get(), Ordering::Relaxed);
                    }
                    SessionId::None => {
                        log::info!("Live microphone has no audio session, preset {preset:?}")
                    }
                }
            }
            Ok(Input(stream, canceller))
        };
        // 20 ms reads, one outgoing frame each.
        let mut buffer = [0.0_f32; (SAMPLE_RATE / 50) as usize];
        log::info!("Microphone opening (16 kHz mono, {preset:?})");
        let result = run_capture(open, &mut *samples, stop, active.as_ref(), &mut buffer);
        if let Some(session) = &session {
            session.current.store(0, Ordering::Relaxed);
        }
        log::info!("Microphone closed");
        result
    }

    /// What live capture with the communication preset needs for its audio session.
    struct Session {
        /// Where the session is published while the stream is open.
        current: Arc<AtomicI32>,
        /// The session to capture in again, and where a new one is kept.
        kept: Arc<AtomicI32>,
        android: Arc<crate::android::Android>,
        /// Whether to enable the echo canceller, or disable it for the echo test.
        canceller: bool,
    }

    /// The stream, and the echo canceller attached to its session while it is open.
    struct Input(
        AudioStream,
        #[expect(
            dead_code,
            reason = "kept for its lifetime: dropping it releases the canceller"
        )]
        Option<crate::android::EchoCanceller>,
    );

    impl InputStream for Input {
        fn start(&mut self) -> Result<(), String> {
            self.0
                .request_start()
                .map_err(|error| format!("Cannot start the microphone: {error}"))
        }

        fn read(&mut self, buffer: &mut [f32], timeout_ns: i64) -> Result<usize, String> {
            // SAFETY: `buffer` holds `buffer.len()` samples of the negotiated mono float format, and only this thread uses the stream.
            let read = unsafe {
                self.0
                    .read(buffer.as_mut_ptr().cast(), buffer.len() as i32, timeout_ns)
            };
            frame_count(read.map_err(i32::from), buffer.len())
        }

        fn stop(&mut self) {
            let _ = self.0.request_stop();
        }

        fn overruns(&self) -> i32 {
            self.0.x_run_count()
        }
    }

    /// The samples read. ndk 0.9 returns a positive count from `AAudioStream_read` as an error code, so only negative values are errors.
    fn frame_count(result: Result<u32, i32>, capacity: usize) -> Result<usize, String> {
        let count = match result {
            Ok(count) => i64::from(count),
            Err(code) => i64::from(code),
        };
        if count < 0 {
            return Err(format!("The microphone failed (AAudio error {count})."));
        }
        if count as usize > capacity {
            return Err("The microphone returned more samples than asked for.".into());
        }
        Ok(count as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Active, CONNECTION_LOST, Capture, Failed, InputStream, MAX_DRAIN_READS, Recording, Samples,
        downmix, run_capture,
    };
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::rc::Rc;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// One scripted device step, run by a read.
    enum Step {
        /// A read that returns these samples, after running a side effect
        /// that happens while the read is in flight.
        Read(Vec<f32>, Box<dyn FnOnce()>),
        Fail,
    }

    #[derive(Default)]
    struct Log {
        started: bool,
        stopped: bool,
        blocking_reads: usize,
        drain_reads: usize,
        overruns: i32,
    }

    struct Fake {
        steps: VecDeque<Step>,
        /// What the device already holds when read without waiting.
        buffered: VecDeque<Result<Vec<f32>, ()>>,
        log: Rc<RefCell<Log>>,
    }

    impl InputStream for Fake {
        fn start(&mut self) -> Result<(), String> {
            self.log.borrow_mut().started = true;
            Ok(())
        }

        fn read(&mut self, buffer: &mut [f32], timeout_ns: i64) -> Result<usize, String> {
            let next = if timeout_ns == 0 {
                self.log.borrow_mut().drain_reads += 1;
                match self.buffered.pop_front() {
                    None => return Ok(0),
                    Some(Err(())) => return Err("drain failed".into()),
                    Some(Ok(samples)) => samples,
                }
            } else {
                self.log.borrow_mut().blocking_reads += 1;
                match self.steps.pop_front().expect("unexpected read") {
                    Step::Read(samples, during) => {
                        during();
                        samples
                    }
                    Step::Fail => return Err("read failed".into()),
                }
            };
            buffer[..next.len()].copy_from_slice(&next);
            Ok(next.len())
        }

        fn stop(&mut self) {
            self.log.borrow_mut().stopped = true;
        }

        fn overruns(&self) -> i32 {
            self.log.borrow().overruns
        }
    }

    fn fake(steps: Vec<Step>, buffered: Vec<Result<Vec<f32>, ()>>) -> (Fake, Rc<RefCell<Log>>) {
        let log = Rc::new(RefCell::new(Log::default()));
        let fake = Fake {
            steps: steps.into(),
            buffered: buffered.into(),
            log: log.clone(),
        };
        (fake, log)
    }

    fn run(
        fake: Fake,
        stop: &Arc<AtomicBool>,
        active: Option<&Active>,
        during_open: impl FnOnce(),
    ) -> (Result<(), String>, Vec<f32>) {
        let mut delivered = Vec::new();
        let mut buffer = [0.0_f32; 8];
        let result = run_capture(
            || {
                during_open();
                Ok(fake)
            },
            &mut |samples: &[f32]| delivered.extend_from_slice(samples),
            stop,
            active,
            &mut buffer,
        );
        (result, delivered)
    }

    #[test]
    fn a_disconnect_while_the_device_opens_never_starts_the_stream() {
        let connected = Arc::new(AtomicBool::new(true));
        let current = connected.clone();
        let active: Active = Box::new(move || current.load(Ordering::SeqCst));
        let (device, log) = fake(vec![], vec![]);
        let stop = Arc::new(AtomicBool::new(false));
        let (result, delivered) = run(device, &stop, Some(&active), || {
            connected.store(false, Ordering::SeqCst)
        });
        assert_eq!(result.unwrap_err(), CONNECTION_LOST);
        assert!(delivered.is_empty());
        assert!(!log.borrow().started);
    }

    #[test]
    fn a_disconnect_before_opening_never_opens_the_device() {
        let active: Active = Box::new(|| false);
        let stop = Arc::new(AtomicBool::new(false));
        let mut opened = false;
        let result = run_capture::<Fake>(
            || {
                opened = true;
                unreachable!()
            },
            &mut |_| panic!("delivered"),
            &stop,
            Some(&active),
            &mut [0.0; 8],
        );
        assert_eq!(result.unwrap_err(), CONNECTION_LOST);
        assert!(!opened);
    }

    #[test]
    fn a_disconnect_during_a_read_drops_that_read_and_stops() {
        let connected = Arc::new(AtomicBool::new(true));
        let current = connected.clone();
        let active: Active = Box::new(move || current.load(Ordering::SeqCst));
        let lost = connected.clone();
        let (device, log) = fake(
            vec![
                Step::Read(vec![0.1; 4], Box::new(|| {})),
                Step::Read(
                    vec![0.2; 4],
                    Box::new(move || lost.store(false, Ordering::SeqCst)),
                ),
            ],
            vec![Ok(vec![0.3; 4])],
        );
        let stop = Arc::new(AtomicBool::new(false));
        let (result, delivered) = run(device, &stop, Some(&active), || {});
        assert_eq!(result.unwrap_err(), CONNECTION_LOST);
        assert_eq!(delivered, vec![0.1; 4]);
        let log = log.borrow();
        assert!(log.stopped);
        assert_eq!(log.drain_reads, 0);
    }

    #[test]
    fn release_during_a_read_delivers_it_and_what_the_device_already_holds() {
        let stop = Arc::new(AtomicBool::new(false));
        let release = stop.clone();
        let (device, log) = fake(
            vec![
                Step::Read(vec![0.1; 8], Box::new(|| {})),
                // Push-to-talk is released while this read is in flight.
                Step::Read(
                    vec![0.2; 8],
                    Box::new(move || release.store(true, Ordering::Release)),
                ),
            ],
            // The device held one full and one partial frame at release.
            vec![Ok(vec![0.3; 8]), Ok(vec![0.4; 3])],
        );
        let (result, delivered) = run(device, &stop, None, || {});
        result.unwrap();
        let mut expected = vec![0.1; 8];
        expected.extend([0.2; 8]);
        expected.extend([0.3; 8]);
        expected.extend([0.4; 3]);
        assert_eq!(delivered, expected);
        let log = log.borrow();
        assert!(log.stopped);
        assert_eq!(log.blocking_reads, 2);
        assert_eq!(log.drain_reads, 3);
    }

    #[test]
    fn a_failed_final_read_fails_capture_instead_of_ending_short() {
        let stop = Arc::new(AtomicBool::new(false));
        let release = stop.clone();
        let (device, _log) = fake(
            vec![Step::Read(
                vec![0.1; 8],
                Box::new(move || release.store(true, Ordering::Release)),
            )],
            vec![Ok(vec![0.2; 2]), Err(())],
        );
        let (result, delivered) = run(device, &stop, None, || {});
        assert_eq!(result.unwrap_err(), "drain failed");
        // What was delivered is still pre-boundary audio; the caller cancels
        // the utterance on failure rather than ending it.
        assert_eq!(delivered.len(), 10);

        let (device, log) = fake(vec![Step::Fail], vec![]);
        let stop = Arc::new(AtomicBool::new(false));
        let (result, delivered) = run(device, &stop, None, || {});
        assert_eq!(result.unwrap_err(), "read failed");
        assert!(delivered.is_empty());
        assert!(log.borrow().stopped);
    }

    #[test]
    fn an_overrun_fails_push_to_talk_but_only_glitches_live_audio() {
        let overrun = |log: &Rc<RefCell<Log>>| {
            let log = log.clone();
            Box::new(move || log.borrow_mut().overruns += 1) as Box<dyn FnOnce()>
        };
        let log = Rc::new(RefCell::new(Log::default()));
        let device = Fake {
            steps: vec![
                Step::Read(vec![0.1; 4], Box::new(|| {})),
                Step::Read(vec![0.2; 4], overrun(&log)),
            ]
            .into(),
            buffered: VecDeque::new(),
            log: log.clone(),
        };
        let stop = Arc::new(AtomicBool::new(false));
        let (result, delivered) = run(device, &stop, None, || {});
        assert!(result.unwrap_err().contains("lost audio"));
        // The samples after the gap are not delivered.
        assert_eq!(delivered, vec![0.1; 4]);

        let log = Rc::new(RefCell::new(Log::default()));
        let release = stop.clone();
        stop.store(false, Ordering::Release);
        let device = Fake {
            steps: vec![
                Step::Read(vec![0.2; 4], overrun(&log)),
                Step::Read(
                    vec![0.3; 4],
                    Box::new(move || release.store(true, Ordering::Release)),
                ),
            ]
            .into(),
            buffered: VecDeque::new(),
            log,
        };
        let active: Active = Box::new(|| true);
        let (result, delivered) = run(device, &stop, Some(&active), || {});
        result.unwrap();
        assert_eq!(delivered.len(), 8);
    }

    #[test]
    fn a_device_that_never_runs_dry_fails_capture() {
        let stop = Arc::new(AtomicBool::new(true));
        let endless = (0..=MAX_DRAIN_READS).map(|_| Ok(vec![0.0; 1])).collect();
        let (device, _log) = fake(vec![], endless);
        // Stop before the first read still starts nothing.
        let (result, delivered) = run(device, &stop, None, || {});
        result.unwrap();
        assert!(delivered.is_empty());

        let stop = Arc::new(AtomicBool::new(false));
        let release = stop.clone();
        let endless = (0..=MAX_DRAIN_READS).map(|_| Ok(vec![0.0; 1])).collect();
        let (device, _log) = fake(
            vec![Step::Read(
                vec![],
                Box::new(move || release.store(true, Ordering::Release)),
            )],
            endless,
        );
        let (result, delivered) = run(device, &stop, None, || {});
        assert!(result.unwrap_err().contains("kept delivering"));
        assert_eq!(delivered.len(), MAX_DRAIN_READS);
    }

    #[test]
    fn downmixes_interleaved_channels() {
        let mut mono = Vec::new();
        downmix(&[1.0, 0.0, 0.5, 0.5, -1.0, -0.5], 2, &mut mono);
        assert_eq!(mono, [0.5, 0.5, -0.75]);
        downmix(&[0.25, 0.75], 1, &mut mono);
        assert_eq!(mono, [0.25, 0.75]);
    }

    #[test]
    fn platforms_without_independently_stoppable_live_capture_refuse_it() {
        struct Unsupported;
        impl Capture for Unsupported {
            fn open(&mut self, _samples: Samples, _failed: Failed) -> Result<Recording, String> {
                panic!("unsupported live capture must not open the microphone")
            }
        }
        let active: Active = Box::new(|| true);
        let error = match Unsupported.open_live(Box::new(|_| {}), Box::new(|_| {}), active) {
            Ok(_) => panic!("live capture unexpectedly opened"),
            Err(error) => error,
        };
        assert!(error.contains("not available"));
    }
}
