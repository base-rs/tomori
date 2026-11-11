// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use std::sync::mpsc::{self, Receiver, SyncSender};

use super::*;
use crate::playback::ambience::Path;
use crate::playback::spatial::Listener;

/// A bus whose first voice the test feeds, and the blocks given back.
fn bus() -> (Bus, SyncSender<Block>, Receiver<Vec<f32>>) {
    let mut receivers = Vec::new();
    let mut first = None;
    for _ in 0..VOICES {
        let (sender, receiver) = mpsc::sync_channel(16);
        first.get_or_insert(sender);
        receivers.push(receiver);
    }
    let (recycle, recycled) = mpsc::sync_channel(64);
    let mut bus = Bus::new(receivers, recycle);
    bus.prepare(1_000);
    (bus, first.unwrap(), recycled)
}

fn spec(kind: Kind, rate: u32, pitch: f32, gain: f32) -> Spec {
    Spec {
        kind,
        rate,
        pitch,
        gain,
    }
}

fn scene(position: [f32; 3]) -> Scene {
    Scene {
        listener: Some(Listener {
            position,
            orientation: [0.0, 0.0, 0.0, 1.0],
        }),
        sources: Vec::new(),
        mono: false,
    }
}

fn mix(bus: &mut Bus, scene: &Scene, layout: Layout, frames: usize) -> Vec<f32> {
    let mut out = vec![0.0; frames * layout.channels()];
    bus.mix(scene, layout, &mut out, &mut Vec::new());
    out
}

fn energy(samples: impl Iterator<Item = f32>) -> f32 {
    samples.map(|sample| sample * sample).sum()
}

#[test]
fn a_bed_plays_its_samples_at_the_device_rate_in_both_ears_or_mixed_down() {
    let (mut bus, sender, recycled) = bus();
    // At twice the device's rate, every other sample, in a straight line between.
    sender
        .send(Block::Begin(spec(Kind::Bed, 2_000, 1.0, 0.5)))
        .unwrap();
    let ramp: Vec<f32> = (0..8)
        .flat_map(|n| [n as f32 * 100.0, -(n as f32) * 100.0])
        .collect();
    sender.send(Block::Samples(ramp)).unwrap();
    let out = mix(&mut bus, &Scene::default(), Layout::Stereo, 3);
    assert_eq!(out, [0.0, 0.0, 100.0, -100.0, 200.0, -200.0]);
    // Mixed down for one channel, the two together, and so in both ears for mono audio.
    let out = mix(&mut bus, &Scene::default(), Layout::Mono, 1);
    assert_eq!(out, [0.0]);
    let mono = Scene {
        mono: true,
        ..Scene::default()
    };
    sender
        .send(Block::Samples(vec![300.0, 100.0, 300.0, 100.0]))
        .unwrap();
    let out = mix(&mut bus, &mono, Layout::Stereo, 1);
    assert_eq!(out, [100.0, 100.0]);
    assert_eq!(bus.playing(), 1);
    // Out of samples, it is late and silent, and goes on once more come, the block it played given back.
    let out = mix(&mut bus, &Scene::default(), Layout::Stereo, 4);
    assert!(out.iter().all(|sample| *sample == 0.0));
    assert_eq!(bus.late(), 1);
    assert_eq!(recycled.try_recv().unwrap().len(), 16);
    sender.send(Block::Samples(vec![1_000.0; 8])).unwrap();
    sender.send(Block::End).unwrap();
    mix(&mut bus, &Scene::default(), Layout::Stereo, 8);
    // Its end frees the voice, and the next sound begins in it.
    assert_eq!(bus.playing(), 0);
    sender
        .send(Block::Begin(spec(Kind::Bed, 1_000, 1.0, 1.0)))
        .unwrap();
    sender.send(Block::Samples(vec![5.0; 4])).unwrap();
    mix(&mut bus, &Scene::default(), Layout::Stereo, 1);
    assert_eq!(bus.playing(), 1);
    // Once the thread has gone, it ends too.
    drop(sender);
    mix(&mut bus, &Scene::default(), Layout::Stereo, 4);
    assert_eq!(bus.playing(), 0);
}

#[test]
fn a_call_is_placed_as_a_voice_is_and_plays_at_its_pitch() {
    let (mut bus, sender, _recycled) = bus();
    sender
        .send(Block::Begin(spec(
            Kind::Call {
                position: [-2.0, 1.6, 0.0],
            },
            1_000,
            2.0,
            1.0,
        )))
        .unwrap();
    sender.send(Block::Samples(vec![1_000.0; 100])).unwrap();
    sender.send(Block::End).unwrap();
    let out = mix(&mut bus, &scene([0.0, 1.6, 0.0]), Layout::Stereo, 100);
    let left = energy(out.iter().step_by(2).copied());
    let right = energy(out.iter().skip(1).step_by(2).copied());
    assert!(left > 2.0 * right, "{left} {right}");
    // At twice its pitch, its hundred samples last fifty of the device's, and then it ends.
    assert!(out[2 * 60..].iter().all(|sample| *sample == 0.0));
    assert_eq!(bus.playing(), 0);
    // In mono, as loud in each.
    sender
        .send(Block::Begin(spec(
            Kind::Call {
                position: [-2.0, 1.6, 0.0],
            },
            1_000,
            1.0,
            1.0,
        )))
        .unwrap();
    sender.send(Block::Samples(vec![1_000.0; 100])).unwrap();
    let mono = mix(&mut bus, &scene([0.0, 1.6, 0.0]), Layout::Mono, 50);
    assert!(energy(mono.into_iter()) > 0.0);
}

/// The energy of a loop along `path`, heard from `at`.
fn heard(path: &[[f32; 3]], radius: f32, at: [f32; 3]) -> f32 {
    let (mut bus, sender, _recycled) = bus();
    let kind = Kind::Loop {
        path: Path::new(path),
        radius,
    };
    sender
        .send(Block::Begin(spec(kind, 1_000, 1.0, 1.0)))
        .unwrap();
    sender.send(Block::Samples(vec![1_000.0; 200])).unwrap();
    energy(mix(&mut bus, &scene(at), Layout::Stereo, 200).into_iter())
}

#[test]
fn a_loop_is_heard_from_its_nearest_point_and_fades_to_nothing_at_its_radius() {
    let river = [[-50.0, 0.0, -10.0], [50.0, 0.0, -10.0]];
    // Walking along the river, it stays as near.
    let beside = heard(&river, 40.0, [0.0, 0.0, 0.0]);
    let along = heard(&river, 40.0, [30.0, 0.0, 0.0]);
    assert!((along / beside - 1.0).abs() < 0.01, "{beside} {along}");
    // Further away, quieter; fading in its last quarter; silent beyond.
    let further = heard(&river, 40.0, [0.0, 0.0, 15.0]);
    assert!(further < beside / 4.0);
    let fading = heard(&river, 40.0, [0.0, 0.0, 25.0]);
    assert!(fading > 0.0 && fading < further / 4.0, "{fading}");
    assert_eq!(heard(&river, 40.0, [0.0, 0.0, 31.0]), 0.0);
    // Without a listener, from the middle, at full.
    let (mut bus, sender, _recycled) = bus();
    let kind = Kind::Loop {
        path: Path::new(&river),
        radius: 0.0,
    };
    sender
        .send(Block::Begin(spec(kind, 1_000, 1.0, 1.0)))
        .unwrap();
    sender.send(Block::Samples(vec![1_000.0; 200])).unwrap();
    let out = mix(&mut bus, &Scene::default(), Layout::Stereo, 10);
    assert!((out[18] - 1_000.0).abs() < 1.0, "{out:?}");
}

#[test]
fn the_bus_makes_its_ears_again_only_for_another_rate() {
    let (mut bus, _sender, _recycled) = bus();
    assert_eq!(bus.rate, 1_000);
    bus.prepare(1_000);
    bus.prepare(48_000);
    assert_eq!(bus.rate, 48_000);
}
