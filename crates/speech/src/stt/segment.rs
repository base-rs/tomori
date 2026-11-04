// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Splitting a long utterance at its pauses, so each part fits the offline recognizer's 30 second window.
//!
//! Whisper decodes at most 30 seconds at once. An utterance longer than [`MAX_SEGMENT`] is decoded as consecutive segments, each ending at the quietest point of the last [`SEARCH`] before its limit, so a cut falls between words rather than through one. A [`VoiceActivity`] detector, when there is one, decides first: a point it hears no speech around is preferred to one it does, and loudness decides between the rest. Only a signal with no pause at all, such as a steady tone or noise, is cut at the limit itself.

use std::ops::Range;

use super::RATE;
use crate::Error;

/// The longest segment passed to the offline recognizer: 28 seconds.
///
/// Whisper's window is 30 seconds, and sherpa-onnx keeps half a second of it for padding after the audio, so 28 seconds leaves room to spare.
pub const MAX_SEGMENT: usize = RATE as usize * 28;

/// How far before a segment's limit to look for a pause to end it at: 10 seconds, so every segment but the last is at least 18 seconds long.
pub const SEARCH: usize = RATE as usize * 10;

/// The resolution at which cuts are placed: 10 ms.
const FRAME: usize = RATE as usize / 100;

/// The stretch of audio around a candidate cut that must be quiet: 250 ms, longer than the closure of a consonant within a word, and shorter than most pauses between phrases.
const PAUSE: usize = RATE as usize / 4;

/// The audio around a cut, besides the search itself, that the voice activity detector hears, so it has context at the edges of the search: 1 second.
const CONTEXT: usize = RATE as usize;

/// A pause must be at least this many times quieter than the typical stretch of the search (6 dB in power); otherwise the signal has no pause, and is cut at the limit.
const QUIETER: f64 = 4.0;

/// Finds speech in audio, so that cuts avoid it.
pub trait VoiceActivity: Send {
    /// The ranges of `samples`, 16 kHz mono, that hold speech, in order and relative to the start of `samples`.
    fn speech(&mut self, samples: &[i16]) -> Result<Vec<Range<usize>>, Error>;
}

impl<V: VoiceActivity + ?Sized> VoiceActivity for Box<V> {
    fn speech(&mut self, samples: &[i16]) -> Result<Vec<Range<usize>>, Error> {
        (**self).speech(samples)
    }
}

/// Where a segment ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cut {
    /// The rest of the clip fits in one segment.
    Rest(usize),
    /// At the middle of the quietest pause before the limit.
    Pause(usize),
    /// At the limit, as the audio before it has no pause.
    Hard(usize),
}

impl Cut {
    /// The index, in the clip, of the first sample after the segment.
    pub fn end(self) -> usize {
        match self {
            Self::Rest(end) | Self::Pause(end) | Self::Hard(end) => end,
        }
    }
}

/// The range of `samples` around the search for the end of the segment starting at `start` that a voice activity detector should hear, or `None` if the rest of the clip fits in one segment.
pub fn listen(samples: &[i16], start: usize) -> Option<Range<usize>> {
    let limit = start + MAX_SEGMENT;
    if samples.len() <= limit {
        return None;
    }
    let first = (limit - SEARCH)
        .saturating_sub(PAUSE / 2 + CONTEXT)
        .max(start);
    let last = (limit + PAUSE / 2 + CONTEXT).min(samples.len());
    Some(first..last)
}

/// Where the segment of `samples` that starts at `start` ends.
///
/// `speech` lists the ranges of `samples` known to hold speech, in clip indices, or is empty when no detector heard them. If the rest of the clip is at most [`MAX_SEGMENT`] long, it is one segment. Otherwise the segment ends at a candidate point, every 10 ms in the last [`SEARCH`] before its limit:
///
/// - If the detector heard speech in the search, and none in the 250 ms around some candidates, the quietest of those.
/// - Otherwise, the candidate with the least energy in the 250 ms around it, if that is markedly quieter than most of the search. A detector that heard speech throughout, as when it merged a short pause into the words around it, or nowhere in the search, does not decide.
/// - Otherwise the audio has no pause, and the segment ends at its limit.
///
/// The latest of equally quiet candidates wins, so segments stay as long as they can.
pub fn cut(samples: &[i16], start: usize, speech: &[Range<usize>]) -> Cut {
    let limit = start + MAX_SEGMENT;
    if samples.len() <= limit {
        return Cut::Rest(samples.len());
    }

    // Prefix sums of energy and of detected speech over the audio any candidate's surroundings can reach.
    let first = (limit - SEARCH).saturating_sub(PAUSE / 2).max(start);
    let last = (limit + PAUSE / 2).min(samples.len());
    let mut energy = Vec::with_capacity(last - first + 1);
    let mut voiced = Vec::with_capacity(last - first + 1);
    let (mut total, mut count) = (0u64, 0u64);
    let mut ranges = speech.iter().peekable();
    energy.push(0);
    voiced.push(0);
    for (index, &sample) in samples.iter().enumerate().take(last).skip(first) {
        let sample = i64::from(sample);
        total += (sample * sample) as u64;
        while ranges.next_if(|range| range.end <= index).is_some() {}
        if ranges.peek().is_some_and(|range| range.contains(&index)) {
            count += 1;
        }
        energy.push(total);
        voiced.push(count);
    }

    // Score each candidate point by the speech and the mean energy around it.
    let candidates: Vec<(usize, u64, f64)> = (0..=SEARCH / FRAME)
        .map(|step| limit - SEARCH + step * FRAME)
        .map(|point| {
            let from = point.saturating_sub(PAUSE / 2).max(first) - first;
            let to = (point + PAUSE / 2).min(last) - first;
            let length = (to - from).max(1) as f64;
            (
                point,
                voiced[to] - voiced[from],
                (energy[to] - energy[from]) as f64 / length,
            )
        })
        .collect();
    // The quietest of `candidates`, the latest of equals.
    let quietest = |candidates: &mut dyn Iterator<Item = &(usize, u64, f64)>| {
        candidates
            .min_by(|a, b| a.2.total_cmp(&b.2).then(b.0.cmp(&a.0)))
            .copied()
    };

    // The detector heard speech in the search, and none around some candidates.
    if candidates.iter().any(|candidate| candidate.1 > 0)
        && let Some((point, _, _)) =
            quietest(&mut candidates.iter().filter(|candidate| candidate.1 == 0))
    {
        return Cut::Pause(point);
    }

    let (point, _, level) =
        quietest(&mut candidates.iter()).expect("the search has at least one candidate");
    let mut levels: Vec<f64> = candidates.iter().map(|candidate| candidate.2).collect();
    levels.sort_by(f64::total_cmp);
    let typical = levels[levels.len() / 2];
    if level * QUIETER <= typical {
        Cut::Pause(point)
    } else {
        Cut::Hard(limit)
    }
}

/// Every segment of `samples`, as found by [`cut`] without a detector: consecutive, in order, and together the whole clip.
pub fn segments(samples: &[i16]) -> Vec<Range<usize>> {
    let mut segments = Vec::new();
    let mut start = 0;
    while start < samples.len() {
        let end = cut(samples, start, &[]).end();
        segments.push(start..end);
        start = end;
    }
    segments
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A deterministic, noise-like signal: a sum of tones that are not multiples of one another, at `amplitude`.
    fn voice(seconds: f64, amplitude: f64) -> Vec<i16> {
        let length = (seconds * f64::from(RATE)) as usize;
        (0..length)
            .map(|index| {
                let time = index as f64 / f64::from(RATE);
                let wave = (time * 2.0 * std::f64::consts::PI * 173.0).sin() * 0.5
                    + (time * 2.0 * std::f64::consts::PI * 419.0).sin() * 0.3
                    + (time * 2.0 * std::f64::consts::PI * 1031.0).sin() * 0.2;
                (wave * amplitude) as i16
            })
            .collect()
    }

    /// `samples` with the range `from..to` seconds made quieter, to `amplitude`.
    fn quieten(samples: &mut [i16], from: f64, to: f64, amplitude: f64) {
        let quiet = voice(to - from, amplitude);
        let start = (from * f64::from(RATE)) as usize;
        samples[start..start + quiet.len()].copy_from_slice(&quiet);
    }

    fn seconds(index: usize) -> f64 {
        index as f64 / f64::from(RATE)
    }

    /// Where `cut` ends a segment at a pause, in seconds, or `None` if it ends it anywhere else.
    fn paused_at(cut: Cut) -> Option<f64> {
        match cut {
            Cut::Pause(end) => Some(seconds(end)),
            Cut::Rest(_) | Cut::Hard(_) => None,
        }
    }

    #[test]
    fn a_clip_that_fits_is_one_segment() {
        let samples = voice(20.0, 8000.0);
        assert_eq!(cut(&samples, 0, &[]), Cut::Rest(samples.len()));
        assert_eq!(segments(&samples), vec![0..samples.len()]);
        // Exactly the limit is still one segment.
        let samples = vec![1000; MAX_SEGMENT];
        assert_eq!(segments(&samples), vec![0..MAX_SEGMENT]);
        assert!(segments(&[]).is_empty());
    }

    #[test]
    fn splits_at_the_pause_before_the_limit() {
        let mut samples = voice(50.0, 8000.0);
        quieten(&mut samples, 22.3, 22.8, 0.0);
        let end = paused_at(cut(&samples, 0, &[])).unwrap();
        assert!((22.4..=22.7).contains(&end), "cut at {end}");
        for segment in segments(&samples) {
            assert!(segment.len() <= MAX_SEGMENT, "{segment:?}");
        }
    }

    #[test]
    fn splits_at_the_quietest_of_several_pauses() {
        let mut samples = voice(60.0, 8000.0);
        // A noisy pause, a quieter one, and a dip too short to be a pause.
        quieten(&mut samples, 19.0, 19.6, 600.0);
        quieten(&mut samples, 21.0, 21.5, 60.0);
        quieten(&mut samples, 25.0, 25.05, 0.0);
        let end = paused_at(cut(&samples, 0, &[])).unwrap();
        assert!((21.1..=21.4).contains(&end), "cut at {end}");
    }

    #[test]
    fn a_pause_outside_the_search_is_not_used() {
        let mut samples = voice(40.0, 8000.0);
        // Too early: a segment ending here would be shorter than it needs to be.
        quieten(&mut samples, 10.0, 11.0, 0.0);
        quieten(&mut samples, 26.0, 26.4, 300.0);
        let end = seconds(cut(&samples, 0, &[]).end());
        assert!((26.0..=26.4).contains(&end), "cut at {end}");
    }

    #[test]
    fn audio_without_a_pause_is_cut_at_the_limit() {
        let samples = voice(70.0, 8000.0);
        assert_eq!(cut(&samples, 0, &[]), Cut::Hard(MAX_SEGMENT));
        assert_eq!(paused_at(cut(&samples, 0, &[])), None);
        assert_eq!(
            segments(&samples),
            vec![
                0..MAX_SEGMENT,
                MAX_SEGMENT..2 * MAX_SEGMENT,
                2 * MAX_SEGMENT..samples.len()
            ]
        );
    }

    #[test]
    fn segments_cover_the_clip_in_order() {
        let mut samples = voice(200.0, 8000.0);
        for pause in [20.0, 45.0, 47.0, 80.0, 130.0, 150.0] {
            quieten(&mut samples, pause, pause + 0.4, 100.0);
        }
        let segments = segments(&samples);
        assert!(segments.len() >= 8, "{segments:?}");
        let mut next = 0;
        for segment in &segments {
            assert_eq!(segment.start, next);
            assert!(
                !segment.is_empty() && segment.len() <= MAX_SEGMENT,
                "{segment:?}"
            );
            next = segment.end;
        }
        assert_eq!(next, samples.len());
    }

    #[test]
    fn detected_speech_outweighs_loudness() {
        let mut samples = voice(40.0, 8000.0);
        // Background noise between words, which the detector hears as no speech, and a soft word, which is quieter but speech.
        quieten(&mut samples, 20.0, 20.5, 2000.0);
        quieten(&mut samples, 24.0, 24.5, 50.0);
        let at = |seconds: f64| (seconds * f64::from(RATE)) as usize;
        let speech = [0..at(19.9), at(20.6)..samples.len()];
        let end = paused_at(cut(&samples, 0, &speech)).unwrap();
        assert!((20.0..=20.5).contains(&end), "cut at {end}");
        // Without the detector, the soft word is the quietest point.
        let end = seconds(cut(&samples, 0, &[]).end());
        assert!((24.0..=24.5).contains(&end), "cut at {end}");
    }

    #[test]
    fn a_detector_that_hears_speech_throughout_leaves_the_cut_to_loudness() {
        let mut samples = voice(40.0, 8000.0);
        quieten(&mut samples, 23.0, 23.5, 0.0);
        // The detector merged the pause into the speech around it, but one point overlaps less speech than the rest.
        let at = |seconds: f64| (seconds * f64::from(RATE)) as usize;
        let speech = [0..at(26.0), at(26.1)..samples.len()];
        let end = paused_at(cut(&samples, 0, &speech)).unwrap();
        assert!((23.0..=23.5).contains(&end), "cut at {end}");
        // With no pause at all, it is cut at the limit, not where the detector heard least speech.
        let samples = voice(40.0, 8000.0);
        assert_eq!(cut(&samples, 0, &speech), Cut::Hard(MAX_SEGMENT));
    }

    #[test]
    fn a_clip_just_longer_than_a_segment_is_cut_at_a_pause_or_the_limit() {
        let samples = voice(28.0, 8000.0);
        let mut longer = samples.clone();
        longer.push(8000);
        assert_eq!(
            segments(&longer),
            vec![0..MAX_SEGMENT, MAX_SEGMENT..MAX_SEGMENT + 1]
        );
        let mut paused = samples;
        quieten(&mut paused, 25.0, 25.4, 0.0);
        paused.push(8000);
        let segments = segments(&paused);
        assert_eq!(segments.len(), 2);
        assert!((25.0..=25.4).contains(&seconds(segments[0].end)));
        assert_eq!(segments[1].end, paused.len());
    }

    #[test]
    fn a_later_segment_is_searched_from_its_own_start() {
        let mut samples = voice(60.0, 8000.0);
        quieten(&mut samples, 23.0, 23.5, 0.0);
        quieten(&mut samples, 45.0, 45.5, 0.0);
        let segments = segments(&samples);
        assert_eq!(segments.len(), 3, "{segments:?}");
        assert!((23.0..=23.5).contains(&seconds(segments[0].end)));
        assert!((45.0..=45.5).contains(&seconds(segments[1].end)));
    }

    #[test]
    fn listens_around_the_search() {
        assert_eq!(listen(&vec![0; MAX_SEGMENT], 0), None);
        let samples = vec![0; MAX_SEGMENT * 3];
        let range = listen(&samples, MAX_SEGMENT).unwrap();
        assert!(range.start < 2 * MAX_SEGMENT - SEARCH && range.start >= MAX_SEGMENT);
        assert!(range.end > 2 * MAX_SEGMENT && range.end <= samples.len());
    }
}
