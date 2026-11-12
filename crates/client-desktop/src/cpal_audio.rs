// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Desktop audio streams' samples and errors, in cpal's terms: mixing a microphone's samples to mono, rendering the player into a speaker's format, and which stream errors lose audio. `desktop_audio.rs` opens the streams with these.
use cpal::{FromSample, Sample};
use tomori_client::{
    capture::Failed,
    speaker::{self, Shared},
};
use tomori_scene::{capture::downmix, playback::render_channels};

/// Mixes a desktop input stream's interleaved samples, in its own format, down to mono samples between -1 and 1. Its buffers are kept between callbacks.
pub struct Mono {
    channels: usize,
    converted: Vec<f32>,
    mono: Vec<f32>,
}

impl Mono {
    pub fn new(channels: u16) -> Self {
        Self {
            channels: usize::from(channels),
            converted: Vec::new(),
            mono: Vec::new(),
        }
    }

    /// `data`, `channels` samples per frame, as mono.
    pub fn mix<T: Sample>(&mut self, data: &[T]) -> &[f32]
    where
        f32: FromSample<T>,
    {
        self.converted.clear();
        self.converted
            .extend(data.iter().map(|sample| sample.to_sample::<f32>()));
        downmix(&self.converted, self.channels, &mut self.mono);
        &self.mono
    }
}

/// Handle a desktop input stream's `error`: some leave it capturing without a gap, and anything else, including an overrun, loses audio, which must not be sent as if whole, and so is `failed`.
pub fn input_failed(error: &cpal::Error, failed: &Failed) {
    match error.kind() {
        cpal::ErrorKind::DeviceChanged | cpal::ErrorKind::RealtimeDenied => {
            log::info!("Microphone: {error}");
        }
        _ => failed(format!("The microphone failed: {error}")),
    }
}

/// Renders the player into a desktop output stream's samples, in its own format, `channels` interleaved samples per frame. Its buffers are kept between callbacks.
pub struct Render {
    player: Shared,
    sample_rate: u32,
    channels: usize,
    samples: Vec<i16>,
    stereo: Vec<i16>,
}

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
    pub fn fill<T: Sample + FromSample<i16>>(&mut self, data: &mut [T]) {
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
pub fn output_failed(error: &cpal::Error, failed: &speaker::Failed) {
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
    use std::sync::{Arc, Mutex, mpsc};
    use tomori_client::playback::{MixerOptions, Player};

    fn player() -> Shared {
        let (_sender, audio) = mpsc::sync_channel(8);
        let (player, _control) = Player::new(audio, MixerOptions::default());
        Arc::new(Mutex::new(player))
    }

    #[test]
    fn desktop_input_in_any_format_is_mixed_to_mono() {
        let mut mono = Mono::new(2);
        assert_eq!(mono.mix(&[1.0_f32, 0.0, -0.5, -0.5]), [0.5, -0.5]);
        assert_eq!(mono.mix(&[i16::MIN, i16::MIN]), [-1.0]);
        let mut single = Mono::new(1);
        assert_eq!(single.mix(&[0_u16]), [-1.0]);
    }

    #[test]
    fn desktop_input_errors_that_lose_audio_fail_the_capture() {
        log::set_max_level(log::LevelFilter::Trace);
        let reported = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = reported.clone();
        let failed: Failed = Box::new(move |error| sink.lock().unwrap().push(error));
        input_failed(&cpal::Error::new(cpal::ErrorKind::DeviceChanged), &failed);
        input_failed(&cpal::Error::new(cpal::ErrorKind::RealtimeDenied), &failed);
        assert!(reported.lock().unwrap().is_empty());
        input_failed(&cpal::Error::new(cpal::ErrorKind::Xrun), &failed);
        let reported = reported.lock().unwrap();
        assert_eq!(reported.len(), 1);
        assert!(reported[0].starts_with("The microphone failed: "));
    }

    #[test]
    fn a_desktop_stream_is_filled_in_its_own_format() {
        let player = player();
        // Nothing to play: silence, in each format's own terms.
        let mut floats = [1.0_f32; 4];
        Render::new(player.clone(), 48_000, 2).fill(&mut floats);
        assert_eq!(floats, [0.0; 4]);
        let mut unsigned = [0_u16; 3];
        Render::new(player, 48_000, 0).fill(&mut unsigned);
        assert_eq!(unsigned, [32_768; 3]);
    }

    #[test]
    fn only_desktop_output_errors_that_stop_it_are_reported() {
        log::set_max_level(log::LevelFilter::Trace);
        let reported = Arc::new(Mutex::new(Vec::new()));
        let sink = reported.clone();
        let failed: speaker::Failed = Box::new(move |error| sink.lock().unwrap().push(error));
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
