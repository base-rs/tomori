// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The beach's stones as a client shows them, from what the farmyard's server side shares: loose bodies ([`Loose`]),
//! shown as any loose body is, every stone where it rests, or between the snapshots of those that move; those this
//! participant holds, one in each hand, taken within reach of it, where the hand closed on it, or pulled to the palm
//! along the ray, and thrown as fast as the hand moved, the heavier slower; those the others hold, drawn from the hands
//! their presence brings; and the clacks and splashes of those that land. See `tomori-environment-farmyard`'s `beach`.
use std::time::Instant;

use glam::{Mat4, Vec2, Vec3};
use tomori_environment_farmyard::beach;
use tomori_physics::Placement;
use tomori_scene::client_side::{Hand, Sound};

pub use crate::loose::{ASTRAY, Loose, Shapes, UNAGREED, placed};

/// How near a hand's grip pose comes to a stone's hull to take it there, rather than pull it along the ray.
pub const REACH: f32 = 0.05;

/// A stone as it is drawn this frame: which, and where.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shown {
    pub stone: usize,
    pub transform: Mat4,
}

/// The beach's stones, as loose bodies are: numbered as the beach numbers them, met by their faces, taken within reach
/// of their hulls, lifted above the shingle, hefted by their weight, clacking and splashing as they land.
#[derive(Clone, Copy, Debug, Default)]
pub struct Beach;

impl Shapes for Beach {
    const PREFIX: &'static str = "stone-";
    const NAME: &'static str = "a river stone";
    const REACH: f32 = REACH;

    fn number(body: u32) -> u32 {
        body
    }

    fn body(number: u32) -> Option<u32> {
        Some(number)
    }

    fn met(origin: Vec3, direction: Vec3, body: u32, placement: &Placement) -> Option<f32> {
        beach::met(origin, direction, body as usize, placement)
    }

    fn reach(body: u32, placement: &Placement, hand: Vec3) -> f32 {
        reach(body, placement, hand)
    }

    /// Within reach, held where the hand closed on it; otherwise pulled to the palm, turned as it was.
    fn held(body: u32, at: &Placement, hand: &Placement, within: bool) -> (Placement, bool) {
        if within {
            (hand.inverse().then(at), false)
        } else {
            (palm(body, hand.rotation.inverse() * at.rotation), true)
        }
    }

    fn lifted(body: u32, placement: Placement) -> Placement {
        lifted(body, placement)
    }

    fn heft(body: u32) -> f32 {
        heft(body)
    }

    fn sound(_body: u32, water: bool, speed: f32) -> Option<Sound> {
        (speed >= QUIET).then(|| sound(water, speed))
    }
}

/// The stones as this client shows them.
pub type Stones = Loose<Beach>;

impl Stones {
    /// Every stone as it is drawn at `now`, the farmyard's clock at `clock`: those this participant holds, in their
    /// hand; one another holds, in theirs, from `right` and `left`, their hands by their numbers, or where it was last
    /// drawn; and the rest where the server side says they are.
    pub fn shown(
        &mut self,
        right: &[(u32, Hand)],
        left: &[(u32, Hand)],
        now: Instant,
        clock: u64,
    ) -> Vec<Shown> {
        self.placed(right, left, now, clock)
            .into_iter()
            .map(|(stone, placement, fade, _)| Shown {
                stone: stone as usize,
                transform: placement.matrix() * Mat4::from_scale(Vec3::splat(fade.scale())),
            })
            .collect()
    }
}

/// How far a hand at `hand` is from the hull of stone `stone` placed at `placement`: from its faces, as
/// [`beach::outside`] measures it, unless it is nowhere near.
fn reach(stone: u32, placement: &Placement, hand: Vec3) -> f32 {
    let (width, height) = beach::size(stone as usize);
    let near = hand.distance(placement.position) - width.max(height) * 0.6;
    if near > REACH {
        return near;
    }
    beach::outside(stone as usize, placement.inverse().transform_point(hand))
}

/// Where stone `stone`, turned `turned` in the hand, is held as it is pulled to it: in front of the fist, along its
/// −Z, as far as the stone reaches back towards the hand that way, a little within it, as the fingers close round it,
/// a fifth of the way and no more than a few centimetres, so that a big stone is held before the hand rather than
/// through it, and a building stone by its near edge.
fn palm(stone: u32, turned: glam::Quat) -> Placement {
    let back = beach::stone(stone as usize)
        .into_iter()
        .map(|point| (turned * point).z)
        .fold(0.0, f32::max);
    let within = (back * 0.2).min(0.04);
    Placement::new(Vec3::new(0.0, 0.0, -(back - within + 0.02)), turned)
}

/// How much of the hand's speed stone `stone` leaves it with, as [`beach::heft`] says: so that a big stone thrown as a
/// small one is goes slower and less far, as it would, and the server side, which holds it to the same, need cut
/// nothing.
pub fn heft(stone: u32) -> f32 {
    beach::heft(stone as usize)
}

/// `placement` of stone `stone`, lifted so that none of it is below the beach's shingle, where there is shingle: so
/// that a stone held is never drawn inside the beach, and is let go on it rather than in it.
fn lifted(stone: u32, placement: Placement) -> Placement {
    let lowest = beach::stone(stone as usize)
        .into_iter()
        .map(|point| placement.transform_point(point))
        .filter_map(|point| {
            beach::height(Vec2::new(point.x, point.z)).map(|shingle| shingle - point.y)
        })
        .fold(0.0, f32::max);
    Placement::new(placement.position + Vec3::Y * lowest, placement.rotation)
}

/// How slowly a stone lands, in metres a second, for nothing to be heard.
pub(crate) const QUIET: f32 = 0.4;

/// The sample rate of the stones' sounds.
pub const RATE: u32 = 48_000;

/// The sound of a stone landing at `speed`, a clack on what it struck, or a splash into water, louder the harder.
pub fn sound(water: bool, speed: f32) -> Sound {
    let level = (speed / 4.0).clamp(0.15, 1.0);
    let samples: Vec<f32> = if water { splash() } else { clack() }
        .iter()
        .map(|sample| sample * level)
        .collect();
    Sound::new(samples, RATE)
}

/// A fixed noise, the same every time, from `seed`.
fn noise(seed: u32) -> impl FnMut() -> f32 {
    let mut state = seed;
    move || {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (state >> 8) as f32 / (1u32 << 23) as f32 - 1.0
    }
}

/// A clack: a few milliseconds of stone striking stone, two ringing tones in a burst of noise, gone in a tenth of a
/// second.
pub fn clack() -> &'static [f32] {
    static CLACK: std::sync::OnceLock<Vec<f32>> = std::sync::OnceLock::new();
    CLACK.get_or_init(|| {
        let mut noise = noise(0x51a7_c1ac);
        (0..RATE as usize / 10)
            .map(|index| {
                let t = index as f32 / RATE as f32;
                let ring = (t * 2_300.0 * std::f32::consts::TAU).sin() * 0.6
                    + (t * 3_700.0 * std::f32::consts::TAU).sin() * 0.4;
                (ring * 0.6 + noise() * 0.4) * (-t * 70.0).exp() * 0.3
            })
            .collect()
    })
}

/// A splash: a burst of water, its noise softened as it falls away, over a third of a second.
pub fn splash() -> &'static [f32] {
    static SPLASH: std::sync::OnceLock<Vec<f32>> = std::sync::OnceLock::new();
    SPLASH.get_or_init(|| {
        let mut noise = noise(0x5b1a_54ed);
        let mut soft = 0.0;
        (0..RATE as usize / 3)
            .map(|index| {
                let t = index as f32 / RATE as f32;
                // A low-pass that closes as it fades, so that the burst turns to a gurgle.
                let open = 0.5 * (-t * 6.0).exp() + 0.05;
                soft += (noise() - soft) * open;
                let attack = (t * 400.0).min(1.0);
                soft * attack * (-t * 9.0).exp() * 0.35
            })
            .collect()
    })
}

#[cfg(test)]
mod tests;
