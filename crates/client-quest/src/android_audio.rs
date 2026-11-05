// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The headset's microphone and speaker through AAudio, behind [`tomori_client::capture::Capture`] and [`tomori_client::speaker::Playback`]. Compiled only for Android.

pub mod capture {
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
    use tomori_client::capture::{Active, Capture, Failed, Recording, Samples};
    use tomori_scene::capture::{InputStream, run_capture};

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

pub mod playback {
    use ndk::audio::{
        AudioCallbackResult, AudioContentType, AudioDirection, AudioFormat, AudioPerformanceMode,
        AudioSharingMode, AudioStreamBuilder, AudioUsage, SessionId,
    };
    use std::num::NonZeroI32;
    use tomori_client::speaker::{Failed, Output, Playback, Route, Shared};
    use tomori_scene::playback::render_channels;

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
