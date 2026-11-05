// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The desktop's default microphone and speaker, opened through cpal. It needs audio hardware, so it is left out of the coverage gate; see the `[[package.metadata.bake.test.coverage.exclude]]` entries in `Cargo.toml`.
//!
//! Only the device calls belong here: what each stream does with its samples and its errors is in [`crate::cpal_audio`], [`crate::capture`] and [`tomori_scene::playback`], where it is tested. See `design/live-audio.md`.

use crate::{capture, cpal_audio};
use cpal::{
    FromSample, SampleFormat, SizedSample, StreamConfig,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};
use tomori_client::{
    capture::{Capture, Failed, Recording, Samples},
    speaker::{Output, Playback, Route, Shared},
};

/// The default input device.
pub struct Microphone;

impl Capture for Microphone {
    fn open(&mut self, samples: Samples, failed: Failed) -> Result<Recording, String> {
        let started = std::time::Instant::now();
        let device = cpal::default_host()
            .default_input_device()
            .ok_or("No microphone is available.")?;
        let supported = device
            .default_input_config()
            .map_err(|error| format!("Cannot use the microphone: {error}"))?;
        let mut config: StreamConfig = supported.config();
        // Ask for the nearest rate the protocol carries; the device refuses it if it cannot.
        config.sample_rate = capture::protocol_rate(config.sample_rate);
        let stream = match supported.sample_format() {
            SampleFormat::F32 => input::<f32>(&device, &config, samples, failed),
            SampleFormat::I16 => input::<i16>(&device, &config, samples, failed),
            SampleFormat::I32 => input::<i32>(&device, &config, samples, failed),
            SampleFormat::U16 => input::<u16>(&device, &config, samples, failed),
            format => Err(format!(
                "The microphone's {format} samples are not supported."
            )),
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

fn input<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    mut samples: Samples,
    failed: Failed,
) -> Result<cpal::Stream, String>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let mut mono = cpal_audio::Mono::new(config.channels);
    device
        .build_input_stream::<T, _, _>(
            *config,
            move |data: &[T], _| samples(mono.mix(data)),
            move |error: cpal::Error| cpal_audio::input_failed(&error, &failed),
            None,
        )
        .map_err(|error| format!("Cannot open the microphone: {error}"))
}

/// The default output device. The desktop has no echo canceller to route for.
pub struct Speaker;

impl Playback for Speaker {
    fn open(&self, player: Shared, failed: Failed, _route: Route) -> Result<Output, String> {
        let device = cpal::default_host()
            .default_output_device()
            .ok_or("No speaker is available.")?;
        let supported = device
            .default_output_config()
            .map_err(|error| format!("Cannot use the speaker: {error}"))?;
        let config: StreamConfig = supported.config();
        // The cues are synthesized for the device's rate here, before the stream starts, rather than in its first callback.
        if let Ok(mut player) = player.lock() {
            player.prepare(config.sample_rate);
        }
        let stream = match supported.sample_format() {
            SampleFormat::F32 => output::<f32>(&device, &config, player, failed),
            SampleFormat::I16 => output::<i16>(&device, &config, player, failed),
            SampleFormat::I32 => output::<i32>(&device, &config, player, failed),
            SampleFormat::U16 => output::<u16>(&device, &config, player, failed),
            format => Err(format!("The speaker's {format} samples are not supported.")),
        }?;
        stream
            .play()
            .map_err(|error| format!("Cannot start the speaker: {error}"))?;
        log::info!(
            "Speaker opened at {} Hz, {} channels",
            config.sample_rate,
            config.channels
        );
        Ok(Output::new(stream))
    }
}

fn output<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    player: Shared,
    failed: Failed,
) -> Result<cpal::Stream, String>
where
    T: SizedSample + FromSample<i16>,
{
    let mut render = cpal_audio::Render::new(player, config.sample_rate, config.channels);
    device
        .build_output_stream::<T, _, _>(
            *config,
            move |data: &mut [T], _| render.fill(data),
            move |error: cpal::Error| cpal_audio::output_failed(&error, &failed),
            None,
        )
        .map_err(|error| format!("Cannot open the speaker: {error}"))
}
