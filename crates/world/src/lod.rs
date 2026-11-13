// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Levels of detail: how coarsely each instance seen is drawn, chosen once a frame for both eyes alike by how many
//! pixels each coarser level's error would span from the nearer eye, with hysteresis, and faded from one level to the
//! next over a few frames rather than switched. See `design/rendering.md`.
use glam::{Mat4, Vec3};

pub use tomori_assets::lod::Lod;

use crate::{Aabb, EYES};

/// The most levels of detail an item is drawn in: its own and three coarser.
pub const LEVELS: usize = 4;

/// The most pixels a coarser level's error, how far its surface strays from its own, may span from the nearer eye for
/// it to be drawn: a pixel, so that what changes is never more than a pixel's edge moving.
pub const TOLERANCE: f32 = 1.0;

/// How much further than [`TOLERANCE`] a level's error must span before a finer level is drawn again, so that an instance
/// at the distance where two levels meet does not change back and forth as the head moves: a quarter, about a metre at
/// 4 m and 5 m at 20 m.
pub const HYSTERESIS: f32 = 1.25;

/// How many frames an instance fades from one level to the next over: about a quarter of a second at 72 Hz.
pub const FADE_FRAMES: u16 = 18;

/// The levels of detail an item's instances may be drawn in, coarser than their own: for each, in order, how far at
/// most its surface strays from their own, in their root's units, each further than the last.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Levels {
    errors: [f32; LEVELS - 1],
    count: u8,
}

impl Levels {
    /// The levels whose errors are `errors`, of at most [`LEVELS`] − 1, each a positive number greater than the last;
    /// those after one that is not are left out.
    pub fn new(errors: &[f32]) -> Self {
        let mut levels = Self::default();
        let mut last = 0.0;
        for error in errors.iter().take(LEVELS - 1) {
            if !(error.is_finite() && *error > last) {
                break;
            }
            levels.errors[levels.count as usize] = *error;
            levels.count += 1;
            last = *error;
        }
        levels
    }

    /// Each coarser level's error, in order.
    pub fn errors(&self) -> &[f32] {
        &self.errors[..self.count as usize]
    }

    /// How many levels there are, the instances' own among them.
    pub fn len(&self) -> usize {
        self.count as usize + 1
    }

    /// Whether there are none coarser than the instances' own.
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// The level to draw an instance in whose nearer eye is `distance` from it, for eyes of `density` pixels a unit of
    /// distance at a unit away, drawn now in `level`: the coarsest whose error spans at most [`TOLERANCE`] pixels, or,
    /// if that is finer, still `level` while its error spans at most [`HYSTERESIS`] times as many.
    pub fn choose(&self, distance: f32, density: f32, level: u8) -> u8 {
        let spans = |error: f32| error * density / distance;
        let coarsest = self
            .errors()
            .iter()
            .rposition(|error| spans(*error) <= TOLERANCE)
            .map_or(0, |index| index as u8 + 1);
        let level = level.min(self.count);
        if coarsest < level && spans(self.errors[level as usize - 1]) <= TOLERANCE * HYSTERESIS {
            level
        } else {
            coarsest
        }
    }
}

/// How an instance's level is changing: settled, fading, `step` frames of [`FADE_FRAMES`] in, or not yet chosen, as
/// when it was not seen last frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Progress {
    #[default]
    Unchosen,
    Settled,
    Fading(u16),
}

/// The next frame's `lod` of an instance seen, `progress` into its change, whose level would be `chosen`: whatever it
/// was not yet chosen is drawn in at once, as it was not seen; a change goes on until it is done, before another is
/// chosen; and a level settled changes by fading from it.
pub(crate) fn step(lod: &mut Lod, progress: &mut Progress, chosen: u8) {
    match *progress {
        Progress::Unchosen => {
            *lod = Lod::settled(chosen);
            *progress = Progress::Settled;
        }
        Progress::Fading(step) if step + 1 >= FADE_FRAMES => {
            *lod = Lod::settled(lod.level);
            *progress = Progress::Settled;
        }
        Progress::Fading(step) => {
            lod.fade = f32::from(step + 1) / f32::from(FADE_FRAMES);
            *progress = Progress::Fading(step + 1);
        }
        Progress::Settled if chosen != lod.level => {
            *lod = Lod {
                level: chosen,
                from: lod.level,
                fade: 1.0 / f32::from(FADE_FRAMES),
            };
            *progress = Progress::Fading(1);
        }
        Progress::Settled => {}
    }
}

/// Where each eye is and how many pixels a unit of distance spans a unit away from it, at the middle of its view, in
/// some coordinates, for eyes `clips` whose images are `height` pixels tall, so that every instance in them chooses its
/// level alike for both eyes.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Eyes {
    eyes: [Vec3; EYES],
    count: usize,
    density: f32,
}

impl Eyes {
    pub(crate) fn new(clips: &[Mat4], height: Option<u32>) -> Self {
        let mut eyes = Self::default();
        let Some(height) = height else {
            return eyes;
        };
        for clip in clips.iter().take(EYES) {
            eyes.eyes[eyes.count] = crate::eye(*clip);
            eyes.count += 1;
            eyes.density = eyes.density.max(density(*clip, height));
        }
        eyes
    }

    /// How far the nearer eye is from `bounds`, none inside it, and how many pixels a unit of distance spans a unit away;
    /// or none, if there are no eyes, nor pixels to measure by, or no bounds.
    pub(crate) fn measure(&self, bounds: Option<&Aabb>) -> Option<(f32, f32)> {
        let bounds = bounds.filter(|bounds| bounds.is_finite() && !bounds.is_empty())?;
        (self.count > 0 && self.density > 0.0).then(|| {
            let distance = self.eyes[..self.count]
                .iter()
                .map(|eye| eye.clamp(bounds.min, bounds.max).distance(*eye))
                .fold(f32::INFINITY, f32::min);
            (distance, self.density)
        })
    }
}

/// How many pixels a unit of distance spans, a unit away, at the middle of the view `clip` of an image `height` pixels
/// tall: its projection's vertical scale, the part of its second row across its depth's, over the length of its depth's,
/// times half the height, as a pixel is as wide as it is tall.
pub(crate) fn density(clip: Mat4, height: u32) -> f32 {
    let row = |index: usize| clip.row(index).truncate();
    let (up, depth) = (row(1), row(3));
    let length = depth.length();
    if length.is_nan() || length <= f32::EPSILON {
        return 0.0;
    }
    let along = depth / length;
    let across = up - along * up.dot(along);
    let density = across.length() / length * height as f32 * 0.5;
    if density.is_finite() { density } else { 0.0 }
}

#[cfg(test)]
mod tests;
