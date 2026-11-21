// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Synthetic head-motion traces and a driver for the tests: deterministic, small and shaped like a participant's presence reaching a spectator, sent every 50 ms and arriving with jitter, drawn at 60 frames a second on a clock the test sets, never the wall's.
use super::{Placed, Presence, Smoother, Smoothing, View};
use glam::{Quat, Vec3};
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};
use tomori_protocol::Pose;

/// A deterministic noise source (xorshift), uniform in −1 to 1.
pub(super) struct Noise(u64);

impl Noise {
    pub fn new(seed: u64) -> Self {
        Self(seed.max(1))
    }

    pub fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 40) as f32 / (1u64 << 23) as f32 - 1.0
    }
}

/// An orientation from yaw about up, pitch about the right and roll about the view axis, in radians, as a head turns.
pub(super) fn orientation(yaw: f32, pitch: f32, roll: f32) -> Quat {
    Quat::from_rotation_y(yaw) * Quat::from_rotation_x(pitch) * Quat::from_rotation_z(roll)
}

/// A head at `position` with `orientation`, as presence carries it.
pub(super) fn pose(position: Vec3, orientation: Quat) -> Pose {
    Pose {
        position: position.to_array(),
        orientation: orientation.to_array(),
    }
}

/// A report of a head alone.
pub(super) fn report(position: Vec3, orientation: Quat) -> Presence {
    Presence {
        head: pose(position, orientation),
        left: None,
        right: None,
        hands: None,
        hidden: false,
    }
}

/// The angle between two orientations, in radians.
pub(super) fn angle(from: Quat, to: Quat) -> f32 {
    from.angle_between(to)
}

/// How far the camera's horizon is tilted, in degrees: the elevation of its right vector above the horizontal plane.
pub(super) fn tilt(orientation: Quat) -> f32 {
    (orientation * Vec3::X)
        .y
        .clamp(-1.0, 1.0)
        .asin()
        .to_degrees()
}

/// The standard deviation of `values`.
pub(super) fn deviation(values: &[f32]) -> f32 {
    let mean = values.iter().sum::<f32>() / values.len() as f32;
    (values
        .iter()
        .map(|value| (value - mean).powi(2))
        .sum::<f32>()
        / values.len() as f32)
        .sqrt()
}

/// Reports `from` to `until` seconds, one every 50 ms, which `head` places at the second it was sent, arriving `latency` later with jitter of up to `jitter`, in the order they were sent.
pub(super) fn arrivals(
    mut head: impl FnMut(f64) -> Placed,
    (from, until): (f64, f64),
    (latency, jitter): (f64, f64),
    seed: u64,
) -> Vec<(f64, Presence)> {
    let mut noise = Noise::new(seed);
    let mut sent = Vec::new();
    let mut previous = f64::MIN;
    let mut at = from;
    while at < until {
        let arrived = (at + latency + f64::from(noise.next()) * jitter).max(previous);
        previous = arrived;
        let placed = head(at);
        sent.push((arrived, report(placed.position, placed.orientation)));
        at += 0.05;
    }
    sent
}

/// A smoother driven on a clock the test sets: reports arrive and frames are drawn at the times it says.
pub(super) struct Run {
    pub smoother: Smoother,
    base: Instant,
    pending: VecDeque<(f64, Presence)>,
}

impl Run {
    pub fn new(mode: Smoothing) -> Self {
        Self {
            smoother: Smoother::new(mode),
            base: Instant::now(),
            pending: VecDeque::new(),
        }
    }

    /// `second` seconds on the test's clock.
    pub fn at(&self, second: f64) -> Instant {
        self.base + Duration::from_secs_f64(second)
    }

    /// Reports to arrive, in the order of their arrival, after those queued.
    pub fn queue(&mut self, reports: impl IntoIterator<Item = (f64, Presence)>) {
        self.pending.extend(reports);
    }

    /// Report `presence` of `source` arriving at `second` now.
    pub fn arrive(&mut self, source: u32, second: f64, presence: Presence) {
        self.smoother.push(source, self.at(second), presence);
    }

    /// Draw `source`'s view at `fps` frames a second from `from` up to `until`, the reports queued arriving, as the first of `source`, as they fall due before each frame.
    pub fn play(&mut self, source: u32, (from, until): (f64, f64), fps: f64) -> Vec<(f64, View)> {
        let mut frames = Vec::new();
        let mut frame = 0u32;
        loop {
            let second = from + f64::from(frame) / fps;
            if second >= until {
                return frames;
            }
            while self
                .pending
                .front()
                .is_some_and(|(arrived, _)| *arrived <= second)
            {
                let (arrived, presence) = self.pending.pop_front().unwrap();
                self.arrive(source, arrived, presence);
            }
            let view = self.smoother.view(Some(source), self.at(second));
            frames.push((second, view.expect("a view of what was reported")));
            frame += 1;
        }
    }
}
