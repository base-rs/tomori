// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;

const RATE: u32 = 48_000;

fn peak(samples: &[f32]) -> f32 {
    samples
        .iter()
        .fold(0.0, |peak, sample| peak.max(sample.abs()))
}

#[test]
fn every_cue_is_short_quiet_and_never_clips() {
    for earcon in Earcon::ALL {
        let spec = earcon.spec();
        let samples = spec.render(RATE);
        // At most a quarter of a second, as `design/feedback.md` asks.
        let length = spec.length();
        assert!((20..=250).contains(&length), "{earcon:?} lasts {length} ms");
        assert_eq!(
            samples.len(),
            length as usize * RATE as usize / 1000,
            "{earcon:?}"
        );
        // Peaks at its level, well below full scale and below speech, so it never clips the mix.
        let level = peak(&samples) / f32::from(i16::MAX);
        assert!(
            (level - spec.level).abs() < 1e-4,
            "{earcon:?} peaks at {level}"
        );
        assert!(spec.level <= 0.15, "{earcon:?}");
        // Starts and ends silently, so it never clicks.
        let (first, last) = (samples[0], samples[samples.len() - 1]);
        assert!(first.abs() < 1.0, "{earcon:?} starts at {first}");
        assert!(last.abs() < 50.0, "{earcon:?} ends at {last}");
    }
}

#[test]
fn cues_are_deterministic_and_distinct() {
    for earcon in Earcon::ALL {
        assert_eq!(earcon.spec().render(RATE), earcon.spec().render(RATE));
    }
    let bank = Bank::new(RATE);
    assert_eq!(bank.sample_rate(), RATE);
    for (index, earcon) in Earcon::ALL.iter().enumerate() {
        for other in &Earcon::ALL[index + 1..] {
            assert_ne!(
                bank.samples(*earcon),
                bank.samples(*other),
                "{earcon:?} and {other:?}"
            );
        }
    }
}

#[test]
fn the_reminder_is_the_request_chime_quieter() {
    let request = Earcon::Request.spec();
    let reminder = Earcon::Reminder.spec();
    assert_eq!(request.notes, reminder.notes);
    assert!(reminder.level < request.level * 0.7);
}

#[test]
fn rising_and_falling_cues_differ_by_pitch_direction() {
    let rises = |earcon: Earcon| {
        let notes = earcon.spec().notes;
        notes.last().unwrap().to > notes[0].from
    };
    assert!(rises(Earcon::On));
    assert!(rises(Earcon::Sent));
    assert!(rises(Earcon::Request));
    assert!(!rises(Earcon::Off));
    assert!(!rises(Earcon::Failed));
    assert!(!rises(Earcon::CatchingUp));
}

#[test]
fn only_the_opening_tick_bypasses_silencing() {
    let bypass: Vec<_> = Earcon::ALL
        .into_iter()
        .filter(|earcon| earcon.bypasses_silence())
        .collect();
    assert_eq!(bypass, [Earcon::On]);
}

#[test]
fn renders_at_any_device_rate() {
    for rate in [8_000, 16_000, 44_100, 96_000] {
        let samples = Earcon::Sent.spec().render(rate);
        assert_eq!(
            samples.len(),
            (190.0 * f64::from(rate) / 1000.0).round() as usize
        );
        assert!(peak(&samples) <= 0.1 * f32::from(i16::MAX) + 1e-3);
    }
}

#[test]
fn a_cue_without_notes_is_silent() {
    let spec = Spec {
        notes: Vec::new(),
        partials: &[(1.0, 1.0)],
        attack: 1,
        level: 0.1,
    };
    assert_eq!(spec.length(), 0);
    assert!(spec.render(RATE).is_empty());
    let silent = Spec {
        notes: vec![Note {
            start: 0,
            length: 10,
            from: 440.0,
            to: 440.0,
        }],
        partials: &[(1.0, 0.0)],
        attack: 1,
        level: 0.1,
    };
    assert!(silent.render(RATE).iter().all(|sample| *sample == 0.0));
}

#[test]
fn the_echo_probe_rises_and_falls_like_speech_at_its_level_and_never_clips() {
    let samples = probe(RATE);
    assert_eq!(samples.len(), 72_000);
    let rms = |part: &[f32]| {
        (part.iter().map(|sample| sample * sample).sum::<f32>() / part.len() as f32).sqrt()
            / f32::from(i16::MAX)
    };
    let level = rms(&samples);
    assert!((level - PROBE_LEVEL).abs() < 0.005, "{level}");
    // Like syllables: louder and quieter four times a second, faded at the ends.
    let syllables: Vec<f32> = samples.chunks(2_400).map(rms).collect();
    let (quietest, loudest) = syllables[2..syllables.len() - 2]
        .iter()
        .fold((f32::MAX, 0.0_f32), |(low, high), level| {
            (low.min(*level), high.max(*level))
        });
    assert!(loudest > 2.5 * quietest, "{syllables:?}");
    assert!(samples[0].abs() < 1.0);
    assert!(peak(&samples) < 0.9 * f32::from(i16::MAX));
    // Only loaded for the echo test.
    let mut bank = Bank::new(RATE);
    assert!(bank.probe().is_empty());
    bank.load_probe();
    bank.load_probe();
    assert_eq!(bank.probe(), &samples[..]);
    assert!(probe(0).len() <= 2);
}

#[test]
fn a_note_rounded_past_the_end_of_its_cue_is_cut_there() {
    // At 2.5 kHz a note of 1 ms starting at 1 ms rounds to three samples starting at the fourth, past the cue's five.
    let spec = Spec {
        notes: vec![Note {
            start: 1,
            length: 1,
            from: 440.0,
            to: 440.0,
        }],
        partials: &[(1.0, 1.0)],
        attack: 0,
        level: 0.1,
    };
    let samples = spec.render(2_500);
    assert_eq!(samples.len(), 5);
    assert_eq!(samples[..4], [0.0; 4]);
    assert!(samples[4] != 0.0);
}
