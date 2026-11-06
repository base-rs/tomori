// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Rendering the player for the platform speaker's device adapters. See `design/live-audio.md`.
//!
//! Each client's speaker is a [`tomori_client::speaker::Playback`], which its own crate implements over the platform's audio interface. Its output callback calls [`render_channels`] with the device's rate and channel count, which never waits for the player, so that the callback cannot stall the device.
use tomori_client::{playback::Layout, speaker::Shared};

/// Render the player at `sample_rate` into `output`, mono or interleaved stereo, or silence if another stream holds it.
fn render(player: &Shared, sample_rate: u32, layout: Layout, output: &mut [i16]) {
    match player.try_lock() {
        Ok(mut player) => player.render(sample_rate, layout, output),
        // A stream that is being replaced, or a poisoned player, plays nothing rather than waiting.
        Err(_) => output.fill(0),
    }
}

/// Render the player at `sample_rate` into `output`, from a device adapter's output callback: `channels` interleaved samples per frame, mono for one channel, and otherwise stereo in the first two with the rest silent. It plays silence, rather than waiting, while another stream holds the player. `stereo` is scratch space, kept between calls.
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
    use std::sync::{Arc, Mutex, mpsc};
    use tomori_client::playback::{MixerOptions, Player, spatial::Listener};
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
