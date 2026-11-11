// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use crate::playback::CEILING;
use crate::playback::duck;
use crate::playback::spatial::Listener;

const RATE: u32 = 1_000;

fn mixer() -> Mixer {
    Mixer::new(MixerOptions {
        sample_rate: RATE,
        jitter: JitterOptions {
            target_latency: Duration::from_millis(4),
            latency_limit: Duration::from_millis(40),
        },
    })
}

fn frame(source: u32, sequence: u32, value: i16) -> AudioFrame {
    AudioFrame {
        gate: Gate::Live,
        source,
        sequence,
        timestamp: 0,
        sample_rate: RATE,
        samples: vec![value; 4],
    }
}

fn mix(mixer: &mut Mixer, length: usize) -> Vec<i16> {
    let mut output = vec![0; length];
    mixer.mix(&mut output);
    output
}

#[test]
fn defaults_suit_a_headset() {
    let options = MixerOptions::default();
    assert_eq!(options.sample_rate, 48_000);
    assert!(options.jitter.target_latency < options.jitter.latency_limit);
    let mut mixer = Mixer::new(options);
    assert_eq!(mixer.agent_volume(), 1.0);
    assert!(!mixer.is_agent_muted());
    assert_eq!(mix(&mut mixer, 480), vec![0; 480]);
}

#[test]
fn sums_participants_and_agent_speech() {
    let mut mixer = mixer();
    mixer.push(frame(1, 0, 100));
    mixer.push(frame(2, 0, 20));
    mixer.push(frame(AGENT_SOURCE, 0, 3));
    assert_eq!(mix(&mut mixer, 4), [123; 4]);
    assert_eq!(mixer.sample_rate(), RATE);
}

#[test]
fn excludes_the_local_microphone_and_gated_off_frames() {
    let mut mixer = mixer();
    mixer.push(frame(5, 0, 1));
    mixer.set_local_source(Some(5));
    assert_eq!(mixer.sources().count(), 0);
    mixer.push(frame(5, 1, 1000));
    let mut off = frame(6, 0, 1000);
    off.gate = Gate::Off;
    mixer.push(off);
    mixer.push(frame(7, 0, 2));
    assert_eq!(mix(&mut mixer, 4), [2; 4]);
    mixer.set_local_source(None);
    mixer.push(frame(5, 2, 9));
    assert_eq!(mixer.sources().count(), 2);
}

#[test]
fn agent_volume_and_mute_are_local_and_only_affect_agent_speech() {
    let mut mixer = mixer();
    mixer.set_agent_volume(0.5);
    assert_eq!(mixer.agent_volume(), 0.5);
    mixer.push(frame(AGENT_SOURCE, 0, 100));
    mixer.push(frame(1, 0, 10));
    assert_eq!(mix(&mut mixer, 4), [60; 4]);

    mixer.set_agent_muted(true);
    assert!(mixer.is_agent_muted());
    mixer.push(frame(AGENT_SOURCE, 1, 100));
    mixer.push(frame(AGENT_SOURCE, 2, 200));
    mixer.push(frame(1, 1, 10));
    assert_eq!(mix(&mut mixer, 4), [10; 4]);

    // Muted speech was consumed in real time, so unmuting resumes with the current frame rather than replaying.
    mixer.set_agent_muted(false);
    mixer.push(frame(1, 2, 10));
    assert_eq!(mix(&mut mixer, 4), [110; 4]);

    mixer.set_agent_volume(-1.0);
    assert_eq!(mixer.agent_volume(), 0.0);
}

#[test]
fn limits_loud_mixes_instead_of_wrapping() {
    let mut mixer = mixer();
    for source in 0..4 {
        mixer.push(frame(source, 0, i16::MAX));
    }
    let output = mix(&mut mixer, 4);
    for sample in output {
        assert!(sample > 0 && f32::from(sample) <= CEILING);
    }
}

#[test]
fn one_late_source_does_not_delay_another() {
    let mut mixer = mixer();
    mixer.push(frame(1, 0, 1));
    // Source 2 has nothing buffered yet; source 1 plays regardless.
    assert_eq!(mix(&mut mixer, 4), [1; 4]);
    mixer.push(frame(2, 0, 2));
    mixer.push(frame(1, 1, 1));
    assert_eq!(mix(&mut mixer, 4), [3; 4]);
}

#[test]
fn restarts_a_source_that_changes_sample_rate() {
    let mut mixer = mixer();
    mixer.push(frame(AGENT_SOURCE, 0, 1));
    let mut faster = frame(AGENT_SOURCE, 1, 4);
    faster.sample_rate = RATE * 2;
    faster.samples = vec![4; 8];
    mixer.push(faster);
    assert_eq!(mix(&mut mixer, 4), [4; 4]);
}

#[test]
fn releases_idle_sources_and_clears() {
    let mut mixer = mixer();
    mixer.push(frame(1, 0, 1));
    mix(&mut mixer, 4);
    assert_eq!(mixer.sources().count(), 1);
    // Ten seconds at 1 kHz.
    mix(&mut mixer, 10_000);
    assert_eq!(mixer.sources().count(), 0);
    assert_eq!(mixer.statistics().underruns, 1);

    mixer.push(frame(1, 1, 1));
    mixer.clear();
    assert_eq!(mixer.sources().count(), 0);

    mixer.push(frame(1, 1, 1));
    mixer.push(frame(2, 1, 1));
    mixer.clear_source(1);
    assert_eq!(mixer.sources().collect::<Vec<_>>(), [2]);
}

#[test]
fn plays_a_source_that_restarts_its_sequence() {
    let mut mixer = mixer();
    for sequence in 500..510 {
        mixer.push(frame(1, sequence, 1));
        mix(&mut mixer, 4);
    }
    for sequence in 0..3 {
        mixer.push(frame(1, sequence, 2));
    }
    assert_eq!(mix(&mut mixer, 12), [2; 12]);
    assert_eq!(mixer.statistics().late_frames, 0);
}

#[test]
fn reports_whether_each_source_is_playing() {
    let mut mixer = mixer();
    assert!(!mixer.is_source_playing(1));
    mixer.push(frame(1, 0, 100));
    mixer.push(frame(1, 1, 100));
    assert!(!mixer.is_source_playing(1), "buffering is not playing");
    mix(&mut mixer, 4);
    assert!(mixer.is_source_playing(1));
    assert!(!mixer.is_source_playing(2));
    // Running out of audio returns the source to buffering.
    mix(&mut mixer, 8);
    assert!(!mixer.is_source_playing(1));
}

#[test]
fn measures_the_level_each_source_played() {
    let mut mixer = mixer();
    assert_eq!(mixer.source_level(1), 0.0);
    let loud = i16::MAX / 2;
    for sequence in 0..4 {
        mixer.push(frame(1, sequence, loud));
        mixer.push(frame(2, sequence, loud / 10));
    }
    mix(&mut mixer, 4);
    let first = mixer.source_level(1);
    let second = mixer.source_level(2);
    assert!((first - 0.5).abs() < 0.01, "{first}");
    assert!((second - 0.05).abs() < 0.01, "{second}");
    assert_eq!(mixer.source_level(3), 0.0);
}

#[test]
fn levels_fall_gradually_when_a_source_goes_quiet() {
    let mut mixer = mixer();
    mixer.push(frame(1, 0, i16::MAX / 2));
    mixer.push(frame(1, 1, i16::MAX / 2));
    mix(&mut mixer, 8);
    let playing = mixer.source_level(1);
    // 30 ms of silence at 1 kHz, less than the release time.
    mix(&mut mixer, 30);
    let falling = mixer.source_level(1);
    assert!(
        falling > 0.0 && falling < playing,
        "{falling} after {playing}"
    );
    // Long after, the level is close to zero.
    mix(&mut mixer, 1_000);
    assert!(mixer.source_level(1) < 0.001);
}

#[test]
fn muted_agent_speech_plays_silently_but_keeps_its_level() {
    let mut mixer = mixer();
    mixer.set_agent_muted(true);
    mixer.push(frame(AGENT_SOURCE, 0, 1_000));
    mixer.push(frame(AGENT_SOURCE, 1, 1_000));
    let output = mix(&mut mixer, 4);
    assert!(output.iter().all(|&sample| sample == 0));
    assert!(mixer.is_source_playing(AGENT_SOURCE));
    assert!(mixer.source_level(AGENT_SOURCE) > 0.0);
}

#[test]
fn a_skip_fades_agent_speech_heard_out_and_its_end_heard_at_once() {
    let mut mixer = mixer();
    for sequence in 0..6 {
        mixer.push(frame(AGENT_SOURCE, sequence, 100));
    }
    assert_eq!(mix(&mut mixer, 4), [100; 4]);
    // Skipped while it is heard, it fades out over 10 ms, 10 samples at 1 kHz, rather than stopping on a step.
    mixer.set_agent_skipped(true);
    let near = |output: Vec<i16>, expected: [i16; 4]| {
        assert!(
            output
                .iter()
                .zip(expected)
                .all(|(&sample, expected)| (sample - expected).abs() <= 1),
            "{output:?}"
        );
    };
    near(mix(&mut mixer, 4), [90, 80, 70, 60]);
    near(mix(&mut mixer, 4), [50, 40, 30, 20]);
    near(mix(&mut mixer, 4), [15, 10, 5, 0]);
    assert_eq!(mix(&mut mixer, 4), [0; 4]);
    // Still played, for its level, though silent here.
    assert!(mixer.is_source_playing(AGENT_SOURCE));
    // Its end, while nothing of it is heard, is heard at once.
    mixer.set_agent_skipped(false);
    mixer.push(frame(AGENT_SOURCE, 6, 100));
    assert_eq!(mix(&mut mixer, 4), [100; 4]);
}

#[test]
fn muting_also_silences_agent_speech_played_again() {
    let mut mixer = mixer();
    mixer.set_agent_muted(true);
    mixer.push(frame(REPLAY_SOURCE, 0, 1_000));
    mixer.push(frame(REPLAY_SOURCE, 1, 1_000));
    mixer.push(frame(1, 0, 500));
    mixer.push(frame(1, 1, 500));
    let output = mix(&mut mixer, 4);
    // Other participants still play.
    assert!(output.iter().all(|&sample| sample == 500));
    assert!(mixer.is_source_playing(REPLAY_SOURCE));
}

#[test]
fn a_source_stays_recent_through_a_short_silence() {
    let mut mixer = mixer();
    assert!(!mixer.is_source_recent(REPLAY_SOURCE, Duration::from_millis(100)));
    mixer.push(frame(REPLAY_SOURCE, 0, 1_000));
    assert!(mixer.is_source_recent(REPLAY_SOURCE, Duration::from_millis(100)));
    mix(&mut mixer, 4);
    // 50 ms after its last audio, it is no longer playing, but still recent.
    mix(&mut mixer, 50);
    assert!(!mixer.is_source_playing(REPLAY_SOURCE));
    assert!(mixer.is_source_recent(REPLAY_SOURCE, Duration::from_millis(100)));
    mix(&mut mixer, 100);
    assert!(!mixer.is_source_recent(REPLAY_SOURCE, Duration::from_millis(100)));
}

#[test]
fn a_source_resampled_to_the_device_rate_ends_once_it_runs_dry() {
    let mut mixer = mixer();
    // Speech at half the device rate ends part way between two of its samples.
    mixer.push(AudioFrame {
        sample_rate: RATE / 2,
        ..frame(REPLAY_SOURCE, 0, 1_000)
    });
    mix(&mut mixer, 8);
    assert!(mixer.is_source_recent(REPLAY_SOURCE, Duration::from_millis(100)));
    mix(&mut mixer, 200);
    assert!(!mixer.is_source_recent(REPLAY_SOURCE, Duration::from_millis(100)));
    // And, idle long enough, it is forgotten.
    mix(&mut mixer, 2 * IDLE_LIMIT.as_millis() as usize);
    assert_eq!(mixer.sources().count(), 0);
}

#[test]
fn clearing_forgets_levels() {
    let mut mixer = mixer();
    mixer.push(frame(1, 0, 1_000));
    mixer.push(frame(2, 0, 1_000));
    mix(&mut mixer, 4);
    mixer.clear_source(1);
    assert_eq!(mixer.source_level(1), 0.0);
    assert!(mixer.source_level(2) > 0.0);
    mixer.clear();
    assert_eq!(mixer.source_level(2), 0.0);
    assert!(!mixer.is_source_playing(2));
}

#[test]
fn plays_sound_cues_locally_without_making_them_sources() {
    let mut mixer = mixer();
    mixer.set_agent_muted(true);
    mixer.play(Earcon::Sent, Direction::Centre);
    assert_eq!(mixer.playing_cues(), 1);
    let expected: Vec<i16> = Bank::new(RATE)
        .samples(Earcon::Sent)
        .iter()
        .map(|sample| sample.round() as i16)
        .collect();
    let output = mix(&mut mixer, expected.len() + 10);
    // Muting agent speech never mutes a cue, and the cue is never a speaker.
    assert_eq!(output[..expected.len()], expected[..]);
    assert_eq!(output[expected.len()..], [0; 10]);
    assert_eq!(mixer.sources().count(), 0);
    assert_eq!(mixer.playing_cues(), 0);
}

#[test]
fn a_cue_continues_across_mixes_and_clearing_does_not_stop_it() {
    let mut mixer = mixer();
    mixer.play(Earcon::Request, Direction::Centre);
    let length = Bank::new(RATE).samples(Earcon::Request).len();
    mix(&mut mixer, length / 2);
    assert_eq!(mixer.playing_cues(), 1);
    mix(&mut mixer, length);
    assert_eq!(mixer.playing_cues(), 0);
    mixer.play(Earcon::Request, Direction::Centre);
    mixer.clear();
    assert_eq!(mixer.playing_cues(), 1);
    assert!(mix(&mut mixer, length).iter().any(|sample| *sample != 0));
}

#[test]
fn at_most_eight_cues_play_and_a_new_one_replaces_the_furthest_through() {
    let mut mixer = mixer();
    mixer.play(Earcon::Request, Direction::Centre);
    mix(&mut mixer, 5);
    for _ in 0..7 {
        mixer.play(Earcon::Knock, Direction::Centre);
    }
    assert_eq!(mixer.playing_cues(), 8);
    mixer.play(Earcon::Choose, Direction::Centre);
    assert_eq!(mixer.playing_cues(), 8);
    // The request chime, furthest through, was replaced: once the knocks end, nothing is left.
    let knock = Bank::new(RATE).samples(Earcon::Knock).len();
    mix(&mut mixer, knock);
    assert_eq!(mixer.playing_cues(), 0);
}

#[test]
fn silencing_fades_out_received_audio_and_cues_but_not_the_opening_tick() {
    let mut mixer = Mixer::new(MixerOptions {
        sample_rate: 48_000,
        ..MixerOptions::default()
    });
    mixer.set_silenced(true);
    // Fully faded after 10 ms.
    let mut output = vec![0; 480];
    mixer.mix(&mut output);
    let bank = Bank::new(48_000);
    for (earcon, heard) in [
        (Earcon::Sent, false),
        (Earcon::Request, false),
        (Earcon::On, true),
        (Earcon::Failed, false),
    ] {
        mixer.play(earcon, Direction::Centre);
        let mut output = vec![0; bank.samples(earcon).len()];
        mixer.mix(&mut output);
        let peak = output.iter().map(|sample| sample.unsigned_abs()).max();
        assert_eq!(peak.unwrap() > 1_000, heard, "{earcon:?}");
    }
    // Unsilenced, playback fades back in over 10 ms, and cues play again.
    let mut faded = Mixer::new(MixerOptions {
        sample_rate: 1_000,
        jitter: JitterOptions {
            target_latency: Duration::ZERO,
            latency_limit: Duration::from_millis(100),
        },
    });
    faded.silence_now();
    for sequence in 0..4 {
        faded.push(AudioFrame {
            gate: Gate::Live,
            source: AGENT_SOURCE,
            sequence,
            timestamp: 0,
            sample_rate: 1_000,
            samples: vec![1_000; 4],
        });
    }
    assert_eq!(mix(&mut faded, 2), [0, 0]);
    faded.set_silenced(false);
    assert_eq!(
        mix(&mut faded, 12)[..10],
        [100, 200, 300, 400, 500, 600, 700, 800, 900, 1_000]
    );
    mixer.set_silenced(false);
    mixer.play(Earcon::Sent, Direction::Centre);
    let mut output = vec![0; bank.samples(Earcon::Sent).len()];
    mixer.mix(&mut output);
    assert!(
        output
            .iter()
            .map(|sample| sample.unsigned_abs())
            .max()
            .unwrap()
            > 1_000
    );
}

fn stereo(mixer: &mut Mixer, frames: usize) -> (Vec<i16>, Vec<i16>) {
    let mut output = vec![0; frames * 2];
    mixer.mix_stereo(&mut output);
    (
        output.iter().step_by(2).copied().collect(),
        output.iter().skip(1).step_by(2).copied().collect(),
    )
}

fn listener() -> Listener {
    Listener {
        position: [0.0, 1.6, 0.0],
        orientation: [0.0, 0.0, 0.0, 1.0],
    }
}

fn energy(channel: &[i16]) -> f64 {
    channel
        .iter()
        .map(|sample| f64::from(*sample).powi(2))
        .sum()
}

#[test]
fn without_a_scene_stereo_plays_the_mono_mix_in_both_ears() {
    let mut mixer = mixer();
    mixer.push(frame(1, 0, 100));
    mixer.push(frame(AGENT_SOURCE, 0, 20));
    let (left, right) = stereo(&mut mixer, 4);
    assert_eq!(left, [120; 4]);
    assert_eq!(right, [120; 4]);
}

#[test]
fn places_each_source_on_its_side_and_agent_speech_played_again_at_the_agent() {
    let mut mixer = Mixer::new(MixerOptions {
        sample_rate: 48_000,
        jitter: JitterOptions {
            target_latency: Duration::ZERO,
            latency_limit: Duration::from_millis(200),
        },
    });
    *mixer.scene_mut() = Scene {
        listener: Some(listener()),
        sources: vec![(1, [-2.0, 1.6, 0.0]), (AGENT_SOURCE, [2.0, 1.6, 0.0])],
        mono: false,
    };
    let frame = |source: u32| AudioFrame {
        gate: Gate::Live,
        source,
        sequence: 0,
        timestamp: 0,
        sample_rate: 48_000,
        samples: vec![1_000; 960],
    };
    mixer.push(frame(1));
    let (left, right) = stereo(&mut mixer, 960);
    assert!(energy(&left) > 5.0 * energy(&right));
    mixer.clear();
    mixer.push(frame(REPLAY_SOURCE));
    let (left, right) = stereo(&mut mixer, 960);
    assert!(energy(&right) > 5.0 * energy(&left));
    // In mono, both ears hear the same.
    mixer.scene_mut().mono = true;
    mixer.clear();
    mixer.push(frame(1));
    let (left, right) = stereo(&mut mixer, 960);
    assert_eq!(left, right);
}

#[test]
fn agent_speech_ducks_while_a_person_speaks_and_keeps_its_level() {
    let mut mixer = mixer();
    let mut sequence = 0;
    let mut play = |mixer: &mut Mixer, person: bool| {
        if person {
            mixer.push(frame(1, sequence, 2_000));
        }
        mixer.push(frame(AGENT_SOURCE, sequence, 1_000));
        sequence += 1;
        mix(mixer, 4)
    };
    // The agent alone, at full volume.
    for _ in 0..3 {
        play(&mut mixer, false);
    }
    assert_eq!(mixer.duck_depth(), 0.0);
    // A person speaking for more than 100 ms ducks it by 8 dB over 80 ms.
    for _ in 0..60 {
        play(&mut mixer, true);
    }
    assert_eq!(mixer.duck_depth(), duck::DEPTH_DB);
    let output = play(&mut mixer, true);
    let agent = f32::from(output[0] - 2_000);
    assert!((agent - 1_000.0 * 0.398).abs() < 2.0, "{agent}");
    // Ducking is not muting: the agent's level is still measured before its gain.
    assert!(mixer.source_level(AGENT_SOURCE) > 0.03);
    // Once the person stops, it comes back up.
    for _ in 0..250 {
        play(&mut mixer, false);
    }
    assert_eq!(mixer.duck_depth(), 0.0);
    assert_eq!(play(&mut mixer, false), [1_000; 4]);
}

#[test]
fn cues_sound_towards_the_hand_or_the_agent() {
    let mut mixer = Mixer::new(MixerOptions::default());
    *mixer.scene_mut() = Scene {
        listener: Some(listener()),
        sources: vec![(AGENT_SOURCE, [2.0, 1.6, -0.1])],
        mono: false,
    };
    let length = Bank::new(48_000).samples(Earcon::Request).len();
    for (direction, louder) in [
        (Direction::Left, Some(true)),
        (Direction::Right, Some(false)),
        (Direction::Agent, Some(false)),
        (Direction::Centre, None),
    ] {
        mixer.play(Earcon::Request, direction);
        let (left, right) = stereo(&mut mixer, length);
        match louder {
            Some(left_louder) => {
                assert_eq!(energy(&left) > energy(&right), left_louder, "{direction:?}")
            }
            None => assert_eq!(left, right),
        }
    }
    // Without the agent placed, or in mono, a cue is centred.
    mixer.scene_mut().sources.clear();
    mixer.play(Earcon::Request, Direction::Agent);
    let (left, right) = stereo(&mut mixer, length);
    assert_eq!(left, right);
    mixer.scene_mut().mono = true;
    mixer.play(Earcon::Request, Direction::Left);
    let (left, right) = stereo(&mut mixer, length);
    assert_eq!(left, right);
    assert_eq!(Layout::Mono.channels(), 1);
}

#[test]
fn silencing_fades_both_ears_together() {
    let mut mixer = mixer();
    for sequence in 0..10 {
        mixer.push(frame(AGENT_SOURCE, sequence, 1_000));
    }
    stereo(&mut mixer, 4);
    mixer.set_silenced(true);
    let (left, right) = stereo(&mut mixer, 12);
    assert_eq!(left, right);
    assert_eq!(left, [900, 800, 700, 600, 500, 400, 300, 200, 100, 0, 0, 0]);
}

#[test]
fn plays_the_echo_probe_once_in_both_ears() {
    let mut mixer = Mixer::new(MixerOptions::default());
    // Not loaded: nothing to play.
    mixer.play_probe();
    assert!(!mixer.is_probing());
    mixer.load_probe();
    mixer.play_probe();
    assert!(mixer.is_probing());
    let length = crate::playback::probe(48_000).len();
    let (left, right) = stereo(&mut mixer, length);
    assert_eq!(left, right);
    assert!(energy(&left) > 0.0);
    assert!(!mixer.is_probing());
    assert_eq!(stereo(&mut mixer, 4).0, [0; 4]);
}

#[test]
fn a_single_loud_frame_does_not_duck_the_agent() {
    let mut mixer = mixer();
    let mut sequence = 0;
    let mut play = |mixer: &mut Mixer, value: i16| {
        mixer.push(frame(1, sequence, value));
        mixer.push(frame(AGENT_SOURCE, sequence, 1_000));
        sequence += 1;
        mix(mixer, 4)
    };
    play(&mut mixer, 0);
    // A 4 ms click, far louder than speech: the speaking indicator holds it for a while, but ducking ignores it.
    play(&mut mixer, 30_000);
    for _ in 0..100 {
        play(&mut mixer, 0);
        assert_eq!(mixer.duck_depth(), 0.0);
    }
}

#[test]
fn without_a_listener_cues_play_in_the_middle() {
    let mut mixer = Mixer::new(MixerOptions::default());
    mixer.play(Earcon::Choose, Direction::Right);
    let length = Bank::new(48_000).samples(Earcon::Choose).len();
    let (left, right) = stereo(&mut mixer, length);
    assert_eq!(left, right);
}

#[test]
fn an_ambient_sound_plays_once_at_the_mixer_s_rate_and_ends() {
    let mut mixer = mixer();
    // At twice the mixer's rate, it lasts half as many of the mixer's samples, in a straight line between its own.
    let sound = Sound::new(
        vec![0.0, 100.0, 200.0, 300.0, 400.0, 500.0, 600.0, 700.0, 800.0],
        RATE * 2,
    );
    mixer.ambient(sound.clone(), [0.0, 0.0, 0.0]);
    assert_eq!(mixer.playing_ambient(), 1);
    assert_eq!(mix(&mut mixer, 2), [0, 200]);
    assert_eq!(mix(&mut mixer, 4), [400, 600, 0, 0]);
    assert_eq!(mixer.playing_ambient(), 0);
    // Nothing at a rate of nothing.
    mixer.ambient(Sound::new(vec![1.0; 4], 0), [0.0; 3]);
    assert_eq!(mixer.playing_ambient(), 0);
    // At most three; another replaces the one furthest through.
    for _ in 0..3 {
        mixer.ambient(Sound::new(vec![10.0; 100], RATE), [0.0; 3]);
        mix(&mut mixer, 1);
    }
    mixer.ambient(sound, [0.0; 3]);
    assert_eq!(mixer.playing_ambient(), 3);
    assert_eq!(mix(&mut mixer, 1), [20]);
}

#[test]
fn an_ambient_sound_never_plays_over_speech() {
    let quiet = || Sound::new(vec![10.0; 100], RATE);
    // A person speaking.
    let mut mixer = mixer();
    for sequence in 0..4 {
        mixer.push(frame(1, sequence, 10_000));
    }
    mix(&mut mixer, 8);
    mixer.ambient(quiet(), [0.0; 3]);
    assert_eq!(mixer.playing_ambient(), 0);
    // The agent speaking, unless it is muted here.
    let mut mixer = self::mixer();
    for sequence in 0..4 {
        mixer.push(frame(AGENT_SOURCE, sequence, 10));
    }
    mix(&mut mixer, 4);
    mixer.ambient(quiet(), [0.0; 3]);
    assert_eq!(mixer.playing_ambient(), 0);
    mixer.set_agent_muted(true);
    mixer.ambient(quiet(), [0.0; 3]);
    assert_eq!(mixer.playing_ambient(), 1);
}

#[test]
fn an_ambient_sound_is_heard_from_where_it_is() {
    let mut mixer = Mixer::new(MixerOptions {
        sample_rate: 48_000,
        jitter: JitterOptions::default(),
    });
    *mixer.scene_mut() = Scene {
        listener: Some(listener()),
        sources: Vec::new(),
        mono: false,
    };
    mixer.ambient(Sound::new(vec![1_000.0; 960], 48_000), [-2.0, 1.6, 0.0]);
    let (left, right) = stereo(&mut mixer, 960);
    assert!(energy(&left) > 2.0 * energy(&right));
}

/// Agent speech as the server paces it (see `crates/server/src/retention.rs` and `voice.rs`): 20 ms frames at 24 kHz,
/// each sent 100 ms before it is due to play, so that those due within 100 ms of the start are sent at once.
#[derive(Clone, Copy, Debug)]
struct Paced {
    source: u32,
    /// The sequence number of the first frame.
    sequence: u32,
    value: i16,
    /// When the first frame is due to play, in milliseconds.
    start: u64,
    frames: u32,
    /// Frames sent before this arrive only then, together, as after a stall of the network.
    late: u64,
}

/// How far ahead of its playback time the server sends each frame.
const LEAD_MS: u64 = 100;
const FRAME_MS: u64 = 20;
const SPEECH_RATE: u32 = 24_000;
const DEVICE_RATE: u32 = 48_000;
/// The headset's output callback, in milliseconds.
const CALLBACK_MS: u64 = 10;

impl Paced {
    fn sent(&self, index: u32) -> u64 {
        (self.start + FRAME_MS * u64::from(index))
            .saturating_sub(LEAD_MS)
            .max(self.start)
    }

    /// Output samples at the device rate this stream plays: a stream resampled at twice its rate keeps its last sample
    /// to interpolate into what comes next, so it plays one fewer.
    fn played(&self) -> usize {
        self.frames as usize * (SPEECH_RATE as usize / 50) * 2 - 1
    }
}

/// Play `streams` through a headset's mixer, at 48 kHz in 10 ms callbacks, for `length` milliseconds, each frame
/// arriving as it is sent; `act` runs each millisecond before what arrives then.
fn play_paced(
    mixer: &mut Mixer,
    streams: &[Paced],
    length: u64,
    mut act: impl FnMut(&mut Mixer, u64),
) -> Vec<i16> {
    let mut output = Vec::new();
    for now in 0..length {
        act(mixer, now);
        for stream in streams {
            for index in
                (0..stream.frames).filter(|&index| stream.sent(index).max(stream.late) == now)
            {
                mixer.push(AudioFrame {
                    gate: Gate::Live,
                    source: stream.source,
                    sequence: stream.sequence + index,
                    timestamp: 0,
                    sample_rate: SPEECH_RATE,
                    samples: vec![stream.value; SPEECH_RATE as usize / 50],
                });
            }
        }
        if now % CALLBACK_MS == 0 {
            output.extend(mix(
                mixer,
                (DEVICE_RATE as u64 * CALLBACK_MS / 1000) as usize,
            ));
        }
    }
    output
}

fn headset() -> Mixer {
    Mixer::new(MixerOptions::default())
}

/// How many output samples hold only `a`, only `b`, and both together, and where the last holding `a` and the first
/// holding `b` are.
#[derive(Debug)]
struct Heard {
    a: usize,
    b: usize,
    both: usize,
    last_a: Option<usize>,
    first_b: Option<usize>,
}

fn heard(output: &[i16], a: i16, b: i16) -> Heard {
    let position = |wanted: &[i16]| output.iter().position(|sample| wanted.contains(sample));
    Heard {
        a: output.iter().filter(|&&sample| sample == a).count(),
        b: output.iter().filter(|&&sample| sample == b).count(),
        both: output.iter().filter(|&&sample| sample == a + b).count(),
        last_a: output
            .iter()
            .rposition(|&sample| sample == a || sample == a + b),
        first_b: position(&[b, a + b]),
    }
}

/// Output samples of `milliseconds` at the device rate.
fn device(milliseconds: u64) -> usize {
    (u64::from(DEVICE_RATE) * milliseconds / 1000) as usize
}

#[test]
fn a_replay_plays_to_its_last_sample_before_live_speech_that_follows_it() {
    const REPLAY: i16 = 1_000;
    const LIVE: i16 = 3_000;
    // A second played again; the server sends its last frame 880 ms in, and the participant hears live speech from
    // then on. The next live segment or reply comes then, or a little later.
    let replay = Paced {
        source: REPLAY_SOURCE,
        sequence: 0,
        value: REPLAY,
        start: 0,
        frames: 50,
        late: 0,
    };
    for after in [0, 20, 40, 80, 120] {
        let live = Paced {
            source: AGENT_SOURCE,
            sequence: 0,
            value: LIVE,
            start: replay.sent(replay.frames - 1) + after,
            frames: 25,
            late: 0,
        };
        let output = play_paced(&mut headset(), &[replay, live], 2_000, |_, _| {});
        let heard = heard(&output, REPLAY, LIVE);
        // Before agent speech was one voice, live speech that came at once played over the last 120 ms of the replay.
        assert_eq!(heard.both, 0, "{after} ms: {heard:?}");
        assert_eq!(heard.a, replay.played(), "{heard:?}");
        assert_eq!(heard.b, live.played(), "{heard:?}");
        assert!(heard.last_a < heard.first_b, "{heard:?}");
        // Live speech that came while the replay played follows its last sample at once, with nothing between them.
        if after < 120 {
            assert_eq!(
                heard.first_b,
                heard.last_a.map(|last| last + 1),
                "{heard:?}"
            );
        }
    }
}

#[test]
fn a_replay_cut_short_by_a_later_reply_plays_what_was_sent_and_then_all_of_the_reply() {
    const REPLAY: i16 = 1_000;
    const LIVE: i16 = 3_000;
    // A later reply arrives half way through a replay, as when an older server stopped the replay there, or a replay
    // is stopped with the reply waiting; the reply is long enough to be held behind what was sent of the replay for as
    // long as it plays, and played whole after it.
    // Its start arrived 100 ms late, together with what followed, so that 220 ms of it are buffered here from then
    // on, as after the network stalls: the reply is held back that long, and more than the buffer's limit of 250 ms of
    // it waits for the replay's end, none of which is discarded.
    let replay = Paced {
        source: REPLAY_SOURCE,
        sequence: 0,
        value: REPLAY,
        start: 0,
        frames: 25,
        late: 100,
    };
    let live = Paced {
        source: AGENT_SOURCE,
        sequence: 0,
        value: LIVE,
        start: replay.sent(replay.frames - 1),
        frames: 100,
        late: 0,
    };
    let mut mixer = headset();
    let output = play_paced(&mut mixer, &[replay, live], 3_000, |_, _| {});
    let heard = heard(&output, REPLAY, LIVE);
    assert_eq!(heard.both, 0, "{heard:?}");
    assert_eq!(heard.a, replay.played(), "{heard:?}");
    assert_eq!(heard.b, live.played(), "{heard:?}");
    assert!(heard.last_a < heard.first_b, "{heard:?}");
    assert_eq!(mixer.statistics().discarded_frames, 0);
}

#[test]
fn a_live_reply_and_the_next_queued_after_it_play_whole_and_in_order() {
    const FIRST: i16 = 1_000;
    const NEXT: i16 = 3_000;
    // Live speech is one stream, whose sequence continues from one reply, or segment, to the next: straight after it,
    // or after a breath.
    for breath in [0, 400] {
        let first = Paced {
            source: AGENT_SOURCE,
            sequence: 0,
            value: FIRST,
            start: 0,
            frames: 50,
            late: 0,
        };
        let next = Paced {
            source: AGENT_SOURCE,
            sequence: 50,
            value: NEXT,
            start: 1_000 + breath,
            frames: 25,
            late: 0,
        };
        let output = play_paced(&mut headset(), &[first, next], 2_500, |_, _| {});
        let heard = heard(&output, FIRST, NEXT);
        assert_eq!(heard.both, 0, "{breath} ms: {heard:?}");
        assert_eq!(heard.a, first.played(), "{breath} ms: {heard:?}");
        // Played straight on, the first reply's last sample leads into the next's first, which is heard whole.
        assert!(heard.b >= next.played(), "{breath} ms: {heard:?}");
        assert!(heard.last_a < heard.first_b, "{breath} ms: {heard:?}");
    }
}

#[test]
fn agent_speech_that_came_first_plays_first() {
    // Live speech still to be heard when a replay starts plays before it, and agent speech is never heard twice at once.
    let mut mixer = mixer();
    mixer.push(frame(AGENT_SOURCE, 0, 10));
    mixer.push(frame(AGENT_SOURCE, 1, 10));
    mixer.push(frame(REPLAY_SOURCE, 0, 30));
    mixer.push(frame(REPLAY_SOURCE, 1, 30));
    // Another participant is not held back.
    mixer.push(frame(1, 0, 100));
    mixer.push(frame(1, 1, 100));
    assert_eq!(mix(&mut mixer, 8), [110; 8]);
    assert!(!mixer.is_source_playing(REPLAY_SOURCE));
    assert_eq!(mix(&mut mixer, 8), [30; 8]);
    assert!(!mixer.is_source_playing(AGENT_SOURCE));
}

#[test]
fn stopping_a_replay_fades_it_out_at_once_and_holds_back_no_live_speech() {
    const REPLAY: i16 = 1_000;
    const LIVE: i16 = 3_000;
    // Two seconds played again, stopped with the right stick a second in. The server stops sending it 20 ms later,
    // when the participant hears live speech, which goes on from then.
    let stop = 1_000;
    let stopped = stop + 20;
    let replay = Paced {
        source: REPLAY_SOURCE,
        sequence: 0,
        value: REPLAY,
        start: 0,
        frames: ((stopped + LEAD_MS) / FRAME_MS) as u32,
        late: 0,
    };
    let live = Paced {
        source: AGENT_SOURCE,
        sequence: 0,
        value: LIVE,
        start: stopped,
        frames: 25,
        late: 0,
    };
    let mut mixer = headset();
    let output = play_paced(&mut mixer, &[replay, live], 2_500, |mixer, now| {
        if now == stop {
            mixer.stop_replay();
        }
    });
    let (before, after) = output.split_at(device(stop));
    assert!(
        before
            .iter()
            .skip(device(LEAD_MS))
            .all(|&sample| sample == REPLAY)
    );
    // It fades out over the next callback, 10 ms, without a step, and is not heard again.
    let fade = &after[..device(CALLBACK_MS)];
    assert!(fade.windows(2).all(|pair| pair[1] <= pair[0]), "{fade:?}");
    assert!(
        fade[0] > REPLAY * 9 / 10 && *fade.last().unwrap() < REPLAY / 10,
        "{fade:?}"
    );
    let rest = &after[device(CALLBACK_MS)..];
    assert!(rest.iter().all(|&sample| sample == 0 || sample == LIVE));
    // Live speech is heard as soon as it arrives, not after the replay's silent end, and whole.
    let first = rest.iter().position(|&sample| sample == LIVE).unwrap();
    assert!(first <= device(stopped - stop), "{first}");
    assert_eq!(
        rest.iter().filter(|&&sample| sample == LIVE).count(),
        live.played()
    );
    // Its silent end over, the next agent speech played again is heard.
    let after = play_paced(
        &mut mixer,
        &[Paced {
            source: REPLAY_SOURCE,
            sequence: replay.frames,
            value: REPLAY,
            start: 0,
            frames: 5,
            late: 0,
        }],
        200,
        |_, _| {},
    );
    assert!(after.contains(&REPLAY));
}

#[test]
fn stopping_a_replay_with_nothing_of_it_to_play_changes_nothing() {
    let mut mixer = mixer();
    mixer.stop_replay();
    // A replay that has run dry, as between its segments, is not stopped either: the server sends no more of it.
    mixer.push(frame(REPLAY_SOURCE, 0, 30));
    assert_eq!(mix(&mut mixer, 8), [30, 30, 30, 30, 0, 0, 0, 0]);
    mixer.stop_replay();
    mixer.push(frame(REPLAY_SOURCE, 1, 30));
    assert_eq!(mix(&mut mixer, 4), [30; 4]);
}

#[test]
fn a_reply_that_waited_for_a_replay_follows_it_whole_after_the_live_speech_before_it() {
    const BEFORE: i16 = 500;
    const REPLAY: i16 = 1_000;
    const LATER: i16 = 3_000;
    // Live speech heard before the replay, then the replay, and then a later reply that started during it, which the
    // server kept until the replay had been sent and then sent after a breath, as the live frames it was: their
    // sequence numbers jump on by what was spoken live meanwhile, and, without a breath, it comes at once.
    for breath in [0, 400] {
        let before = Paced {
            source: AGENT_SOURCE,
            sequence: 0,
            value: BEFORE,
            start: 0,
            frames: 25,
            late: 0,
        };
        let replay = Paced {
            source: REPLAY_SOURCE,
            sequence: 0,
            value: REPLAY,
            start: 700,
            frames: 50,
            late: 0,
        };
        let later = Paced {
            source: AGENT_SOURCE,
            sequence: 1_000,
            value: LATER,
            start: 1_700 + breath,
            frames: 25,
            late: 0,
        };
        let mut mixer = headset();
        let output = play_paced(&mut mixer, &[before, replay, later], 3_000, |_, _| {});
        let count = |value: i16| output.iter().filter(|&&sample| sample == value).count();
        assert_eq!(count(BEFORE), before.played(), "{breath} ms");
        assert_eq!(count(REPLAY), replay.played(), "{breath} ms");
        assert_eq!(count(LATER), later.played(), "{breath} ms");
        let last = |value: i16| output.iter().rposition(|&sample| sample == value);
        let first = |value: i16| output.iter().position(|&sample| sample == value);
        assert!(last(BEFORE) < first(REPLAY), "{breath} ms");
        assert!(last(REPLAY) < first(LATER), "{breath} ms");
        // No two of them together: what else is heard is only the one sample the live stream interpolates half way from
        // its last before the replay to its first after it.
        let between = output
            .iter()
            .filter(|sample| ![0, BEFORE, REPLAY, LATER].contains(*sample))
            .collect::<Vec<_>>();
        assert_eq!(between, [&((BEFORE + LATER) / 2)], "{breath} ms");
        assert_eq!(mixer.statistics().discarded_frames, 0);
    }
}

#[test]
fn the_ambience_bus_plays_ambient_sounds_at_the_participant_s_level_gliding_to_it() {
    let mut mixer = mixer();
    assert_eq!(mixer.ambience_gain(), 1.0);
    assert_eq!(mixer.playing_ambience(), (0, 0));
    mixer.set_ambience_gain(-1.0);
    assert_eq!(mixer.ambience_gain(), 0.0);
    mixer.set_ambience_gain(0.5);
    // The first mix glides from where it was, and the next is at the level.
    mixer.ambient(Sound::new(vec![100.0; 100], RATE), [0.0; 3]);
    let glide = mix(&mut mixer, 4);
    assert!(glide[0] < 100 && glide[3] == 50, "{glide:?}");
    assert_eq!(mix(&mut mixer, 2), [50, 50]);
    // Off, it is silent.
    mixer.set_ambience_gain(0.0);
    mix(&mut mixer, 1);
    assert_eq!(mix(&mut mixer, 2), [0, 0]);
}

#[test]
fn the_ambience_ducks_under_anyone_speaking_but_agent_speech_not_heard_here() {
    /// `source` speaking for 300 ms, as it arrives.
    fn speak(mixer: &mut Mixer, source: u32) {
        for sequence in 0..75 {
            mixer.push(frame(source, sequence, 10_000));
            mix(mixer, 4);
        }
    }
    // A person speaking, over an ambient sound that began before them.
    let mut mixer = mixer();
    mixer.ambient(Sound::new(vec![1_000.0; 2_000], RATE), [0.0; 3]);
    speak(&mut mixer, 1);
    assert_eq!(mixer.ambience_duck_depth(), 12.0);
    // Agent speech, live or played again.
    for source in [AGENT_SOURCE, REPLAY_SOURCE] {
        let mut mixer = self::mixer();
        speak(&mut mixer, source);
        assert_eq!(mixer.ambience_duck_depth(), 12.0);
    }
    // Agent speech muted here, or skipped, does not duck it.
    for muted in [true, false] {
        let mut mixer = self::mixer();
        if muted {
            mixer.set_agent_muted(true);
        } else {
            mixer.set_agent_skipped(true);
        }
        speak(&mut mixer, AGENT_SOURCE);
        assert_eq!(mixer.ambience_duck_depth(), 0.0);
    }
    // Ducked, the ambient sound is 12 dB down, beneath the person still speaking.
    mixer.push(frame(1, 75, 10_000));
    let ducked = mix(&mut mixer, 4);
    assert!(
        ducked
            .iter()
            .all(|sample| (*sample - 10_000 - 251).abs() <= 2),
        "{ducked:?}"
    );
}

#[test]
fn the_ambience_its_thread_feeds_plays_in_the_mix_beneath_cues() {
    use crate::playback::ambience::engine::{self, Command};
    use crate::playback::ambience::{Ambience, Loop, Slot};
    let (mut feeder, bus) = engine::pair(1);
    feeder.apply(Command::Gain(1.0), Duration::ZERO);
    let tone = Slot::embedded("tone", crate::playback::ambience::engine::tests::MONO);
    feeder.apply(
        Command::Ambience(Some(std::sync::Arc::new(
            Ambience::new().with_loop(Loop::at(tone, [0.0; 3])),
        ))),
        Duration::ZERO,
    );
    let mut mixer = Mixer::new(MixerOptions::default());
    mixer.set_ambience_bus(Some(Box::new(bus)));
    feeder.pump(Duration::ZERO);
    let mut output = vec![0; 960];
    mixer.mix_stereo(&mut output);
    feeder.pump(Duration::ZERO);
    mixer.mix_stereo(&mut output);
    assert!(output.iter().any(|sample| sample.unsigned_abs() > 1_000));
    assert_eq!(mixer.playing_ambience(), (1, 0));
    assert!(mixer.take_ambience_bus().is_some());
    assert_eq!(mixer.playing_ambience(), (0, 0));
}
