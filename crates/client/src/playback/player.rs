// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use tomori_protocol::{AGENT_SOURCE, AudioFrame, REPLAY_SOURCE};

use super::ambience::{bus::Bus, engine};
use super::spatial::{Position, Scene};
use super::{Direction, Earcon, Layout, Mixer, MixerOptions, PlaybackStatistics, Sound};

/// How long after its last audio agent speech played again still counts as playing: longer than the breath between spoken segments, so that a replay counts as playing throughout.
const REPLAY_PAUSE: Duration = Duration::from_millis(700);

enum Command {
    LocalSource(Option<u32>),
    Clear,
    AgentMuted(bool),
    AgentSkipped(bool),
    StopReplay,
    Play(Earcon, Direction),
    Ambient(Sound, Position),
    Probe,
    /// The ambience's bus, from its thread, which starts with the first ambience.
    AmbienceBus(Box<Bus>),
    AmbienceGain(f32),
}

/// The scene the application last set, until the callback takes it.
#[derive(Default)]
struct SceneSlot {
    scene: Scene,
    fresh: bool,
}

#[derive(Default)]
struct Shared {
    silenced: AtomicBool,
    /// Whether agent speech played again for this participant was playing at the last render.
    replaying: AtomicBool,
    /// Whether agent speech, live or played again, was playing at the last render.
    speaking: AtomicBool,
    /// The level of agent speech at the last render, as the bits of an `f32`.
    level: AtomicU32,
    statistics: Mutex<PlaybackStatistics>,
    scene: Mutex<SceneSlot>,
    /// The participant's ambience level, as the bits of an `f32` gain, for the ambience's thread as it starts.
    ambience_gain: AtomicU32,
}

/// Plays what a [`crate::Client`] receives: drains its audio into a [`Mixer`] and renders the mix for an output device.
///
/// The player belongs to the audio device's callback, and [`PlayerControl`] to the application. Control changes travel through a channel that [`Self::render`] drains without blocking, so the application never holds anything the callback waits for. Rendering never blocks: missing audio is silence.
///
/// The mixer is created at the device rate by [`Self::prepare`], which an output calls before it starts, since creating one synthesizes the sound cues; [`Self::render`] creates it again if the device rate changes, keeping the local source.
///
/// Unlike a [`Mixer`] shared with the application, the player also calls [`Mixer::push`] from the callback, so the callback allocates and frees: a buffer for each new source, and each frame's samples once played. Received frames are 20 ms each, so this is a few small allocations per callback; nothing waits on a lock.
pub struct Player {
    audio: Receiver<AudioFrame>,
    commands: Receiver<Command>,
    shared: Arc<Shared>,
    options: MixerOptions,
    mixer: Option<Mixer>,
    local_source: Option<u32>,
    agent_muted: bool,
    /// Whether the participant spoke over the agent, so that the rest of what it was saying is skipped here, as muted speech is.
    agent_skipped: bool,
    /// Whether mixers load the echo probe.
    probe: bool,
    /// The ambience level, for mixers made later.
    ambience_gain: f32,
}

/// The ambience's thread: what tells it, and the thread, which ends once that goes.
struct Ambience {
    commands: mpsc::Sender<engine::Command>,
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "only tests join the thread; elsewhere it ends on its own as its commands go"
        )
    )]
    thread: std::thread::JoinHandle<()>,
}

/// The application's handle on a [`Player`]. Cloneable, and never blocks the player.
#[derive(Clone)]
pub struct PlayerControl {
    commands: mpsc::Sender<Command>,
    shared: Arc<Shared>,
    /// The ambience's thread, once an ambience has been set, which ends as the last control goes.
    ambience: Arc<Mutex<Option<Ambience>>>,
}

impl Player {
    /// Play `audio`, typically from [`crate::Client::take_audio`]. `options.sample_rate` is replaced by the rate given to [`Self::render`].
    pub fn new(audio: Receiver<AudioFrame>, options: MixerOptions) -> (Self, PlayerControl) {
        let (commands, receiver) = mpsc::channel();
        let shared = Arc::new(Shared {
            ambience_gain: AtomicU32::new(1.0_f32.to_bits()),
            ..Shared::default()
        });
        let player = Self {
            audio,
            commands: receiver,
            shared: shared.clone(),
            options,
            mixer: None,
            local_source: None,
            agent_muted: false,
            agent_skipped: false,
            probe: false,
            ambience_gain: 1.0,
        };
        let control = PlayerControl {
            commands,
            shared,
            ambience: Arc::default(),
        };
        (player, control)
    }

    /// Synthesize the echo probe now and for later mixers, outside the audio callback, so that [`PlayerControl::play_probe`] can play it. Only the echo test needs it.
    pub fn load_probe(&mut self) {
        self.probe = true;
        if let Some(mixer) = &mut self.mixer {
            mixer.load_probe();
        }
    }

    /// Make the mixer for `sample_rate` now, outside the audio callback, since making one synthesizes every sound cue. An output calls this with the device's rate before it starts; [`Self::render`] makes one only if the rate changes after that.
    pub fn prepare(&mut self, sample_rate: u32) {
        if sample_rate == 0 || self.mixer.as_ref().map(Mixer::sample_rate) == Some(sample_rate) {
            return;
        }
        let mut mixer = Mixer::new(MixerOptions {
            sample_rate,
            ..self.options
        });
        // The ambience plays on in the new mixer, as its thread feeds it.
        mixer.set_ambience_bus(self.mixer.as_mut().and_then(Mixer::take_ambience_bus));
        mixer.set_ambience_gain(self.ambience_gain);
        mixer.set_local_source(self.local_source);
        mixer.set_agent_muted(self.agent_muted);
        // A mixer made while agent speech is skipped starts with it silent, rather than fading out what is not heard.
        mixer.set_agent_skipped(self.agent_skipped);
        if self.probe {
            mixer.load_probe();
        }
        // A mixer made while push-to-talk records starts silent, rather than fading out what the microphone would hear.
        if self.shared.silenced.load(Ordering::Relaxed) {
            mixer.silence_now();
        }
        self.mixer = Some(mixer);
    }

    /// Fill `output` with the next samples at `sample_rate`, mono or interleaved stereo as `layout` says, after taking every frame received since the last call and the latest scene.
    pub fn render(&mut self, sample_rate: u32, layout: Layout, output: &mut [i16]) {
        if sample_rate == 0 {
            output.fill(0);
            return;
        }
        self.prepare(sample_rate);
        let mixer = self.mixer.as_mut().expect("prepared above");
        while let Ok(command) = self.commands.try_recv() {
            match command {
                Command::LocalSource(source) => {
                    self.local_source = source;
                    mixer.set_local_source(source);
                }
                Command::AgentMuted(muted) => {
                    self.agent_muted = muted;
                    mixer.set_agent_muted(muted);
                }
                Command::AgentSkipped(skipped) => {
                    self.agent_skipped = skipped;
                    mixer.set_agent_skipped(skipped);
                }
                Command::StopReplay => mixer.stop_replay(),
                Command::Play(earcon, direction) => mixer.play(earcon, direction),
                Command::Ambient(sound, position) => mixer.ambient(sound, position),
                Command::Probe => mixer.play_probe(),
                Command::AmbienceBus(bus) => mixer.set_ambience_bus(Some(bus)),
                Command::AmbienceGain(gain) => {
                    self.ambience_gain = gain;
                    mixer.set_ambience_gain(gain);
                }
                Command::Clear => {
                    mixer.clear();
                    // Including what arrived before the clear but was not yet rendered, such as a backlog from while no output played.
                    while self.audio.try_recv().is_ok() {}
                }
            }
        }
        while let Ok(frame) = self.audio.try_recv() {
            mixer.push(frame);
        }
        // The latest scene, swapped rather than copied, so that the callback never allocates for it; one the application is writing is taken next time.
        let slot = match self.shared.scene.try_lock() {
            Ok(slot) => Some(slot),
            Err(std::sync::TryLockError::Poisoned(poisoned)) => Some(poisoned.into_inner()),
            Err(std::sync::TryLockError::WouldBlock) => None,
        };
        if let Some(mut slot) = slot
            && slot.fresh
        {
            std::mem::swap(mixer.scene_mut(), &mut slot.scene);
            slot.fresh = false;
        }
        // Silenced audio is still mixed, so that it is consumed in real time and resumes live rather than replaying what was missed.
        mixer.set_silenced(self.shared.silenced.load(Ordering::Relaxed));
        match layout {
            Layout::Mono => mixer.mix(output),
            Layout::Stereo => mixer.mix_stereo(output),
        }
        self.shared.replaying.store(
            mixer.is_source_recent(REPLAY_SOURCE, REPLAY_PAUSE),
            Ordering::Relaxed,
        );
        // Muted or silenced speech is still consumed and measured, so it still plays here.
        self.shared.speaking.store(
            mixer.is_source_playing(AGENT_SOURCE) || mixer.is_source_playing(REPLAY_SOURCE),
            Ordering::Relaxed,
        );
        self.shared.level.store(
            mixer
                .source_level(AGENT_SOURCE)
                .max(mixer.source_level(REPLAY_SOURCE))
                .to_bits(),
            Ordering::Relaxed,
        );
        // Only the application reads the statistics, rarely, so the callback skips publishing them rather than wait.
        if let Ok(mut statistics) = self.shared.statistics.try_lock() {
            *statistics = mixer.statistics();
        }
    }
}

impl PlayerControl {
    /// Never play audio attributed to this participant number: a client does not hear its own microphone. Set from [`crate::ConnectionState::Connected`].
    pub fn set_local_source(&self, source: Option<u32>) {
        // Fails only once the player is gone, when nothing plays.
        let _ = self.commands.send(Command::LocalSource(source));
    }

    /// Discard buffered audio when what the participant hears changes context: on leaving a space, after reconnecting, and when an output starts after nothing played. Every frame received before the next render is discarded too, including any that arrived just after this call.
    pub fn clear(&self) {
        let _ = self.commands.send(Command::Clear);
    }

    /// Mute or unmute agent speech locally, live or played again, while other participants still play. Muted speech is still consumed in real time, so unmuting resumes it live. See `design/speech.md`.
    pub fn set_agent_muted(&self, muted: bool) {
        let _ = self.commands.send(Command::AgentMuted(muted));
    }

    /// Skip the rest of what the agent is saying here, live or played again, or stop skipping it: the participant spoke over it. Skipped speech is silent, as muted speech is, and still consumed in real time and measured, so it still plays; the mute and the skip are kept apart, so that ending one leaves the other. See `design/speech.md`.
    pub fn set_agent_skipped(&self, skipped: bool) {
        let _ = self.commands.send(Command::AgentSkipped(skipped));
    }

    /// Stop agent speech played again here, as the participant asks the server to stop it: what is buffered of it fades out over a few milliseconds, rather than playing on until it runs dry, and holds back no live speech. See [`Mixer::stop_replay`].
    pub fn stop_replay(&self) {
        let _ = self.commands.send(Command::StopReplay);
    }

    /// Whether agent speech played again for this participant, a replay or catch-up, was playing at the last render, including the breaths between its segments.
    pub fn is_replaying(&self) -> bool {
        self.shared.replaying.load(Ordering::Relaxed)
    }

    /// Whether agent speech, live or played again, was playing at the last render, rather than buffering or absent, and its level then, from 0 to 1. Speech muted locally, or silenced while push-to-talk records, is still consumed in real time, so it still plays. Unlike [`Self::is_replaying`], a pause or a buffer running dry stops it at once; the swarm holds across them. See `design/speech.md`.
    pub fn agent_speech(&self) -> (bool, f32) {
        (
            self.shared.speaking.load(Ordering::Relaxed),
            f32::from_bits(self.shared.level.load(Ordering::Relaxed)),
        )
    }

    /// Play a sound cue, once, from `direction`. It is local: never sent and never a source, and [`Self::clear`] does not stop it.
    pub fn play(&self, earcon: Earcon, direction: Direction) {
        let _ = self.commands.send(Command::Play(earcon, direction));
    }

    /// Play an environment's `sound`, once, from `position` in the scene, unless someone is speaking. See [`Mixer::ambient`].
    pub fn ambient(&self, sound: Sound, position: Position) {
        let _ = self.commands.send(Command::Ambient(sound, position));
    }

    /// Play the echo probe, once [`Player::load_probe`] has rendered it, for the echo test to measure what the microphone hears of it.
    pub fn play_probe(&self) {
        let _ = self.commands.send(Command::Probe);
    }

    /// Play `ambience` beneath everything else, or none: its slots load and its sounds decode on a thread of its own,
    /// which starts with the first ambience and ends as the last control goes, and the callback mixes them on its
    /// ambience bus. A thread that cannot start leaves it silent, saying so. See [`super::ambience`].
    pub fn set_ambience(&self, ambience: Option<Arc<super::ambience::Ambience>>) {
        let mut thread = self.ambience.lock().unwrap_or_else(PoisonError::into_inner);
        if thread.is_none() {
            if ambience.is_none() {
                return;
            }
            let seed = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos() as u64;
            let (feeder, bus) = engine::pair(seed);
            let (sender, commands) = mpsc::channel();
            let gain = f32::from_bits(self.shared.ambience_gain.load(Ordering::Relaxed));
            let _ = sender.send(engine::Command::Gain(gain));
            match crate::system::spawn("tomori-ambience", move || engine::run(feeder, commands)) {
                Ok(started) => {
                    let _ = self.commands.send(Command::AmbienceBus(Box::new(bus)));
                    *thread = Some(Ambience {
                        commands: sender,
                        thread: started,
                    });
                }
                Err(error) => {
                    log::warn!("Ambience: cannot start its thread, so it is silent: {error}");
                    return;
                }
            }
        }
        let Some(running) = thread.as_ref() else {
            unreachable!("its thread started above, or before");
        };
        let _ = running.commands.send(engine::Command::Ambience(ambience));
    }

    /// Play the ambience at `gain`, the participant's level: at 0 its thread decodes nothing.
    pub fn set_ambience_gain(&self, gain: f32) {
        let gain = gain.max(0.0);
        self.shared
            .ambience_gain
            .store(gain.to_bits(), Ordering::Relaxed);
        let _ = self.commands.send(Command::AmbienceGain(gain));
        if let Some(running) = self
            .ambience
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
        {
            let _ = running.commands.send(engine::Command::Gain(gain));
        }
    }

    /// End the ambience's thread and wait for it, for tests that check how it ends.
    #[cfg(test)]
    fn end_ambience(&self) -> bool {
        let running = self
            .ambience
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        running.is_some_and(|running| {
            drop(running.commands);
            running.thread.join().is_ok()
        })
    }

    /// Set where the listener and the sources are, typically once a frame. `change` must write the whole scene: it is given an older one to reuse its storage, which may be one the callback has finished with. The callback takes only the latest, at its next render, and never waits for this.
    pub fn set_scene(&self, change: impl FnOnce(&mut Scene)) {
        let mut slot = self
            .shared
            .scene
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        change(&mut slot.scene);
        slot.fresh = true;
    }

    /// Silence playback, fading out over a few milliseconds, while still consuming what arrives in real time; the cues that [`Earcon::bypasses_silence`] still play. Used while the microphone records push-to-talk, so that playback cannot be picked up and transcribed; see `design/live-audio.md`.
    pub fn set_silenced(&self, silenced: bool) {
        self.shared.silenced.store(silenced, Ordering::Relaxed);
    }

    pub fn is_silenced(&self) -> bool {
        self.shared.silenced.load(Ordering::Relaxed)
    }

    /// How playback has handled imperfect input, as of a recent render.
    pub fn statistics(&self) -> PlaybackStatistics {
        *self
            .shared
            .statistics
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests;
