// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! This frame's eyes, and the one frustum that holds all they see.
use glam::{Mat4, Vec3, Vec4};

use crate::{Aabb, Frustum};

/// The most eyes a frame is drawn from: a headset's two.
pub const EYES: usize = 2;

/// Where the eye is, from its view and projection `clip`: the point every ray of the view begins from, or the origin for
/// a view that has none, such as an orthographic one, or that cannot be inverted.
pub fn eye(clip: Mat4) -> Vec3 {
    let origin = clip.inverse() * Vec4::Z;
    if origin.w.abs() > f32::EPSILON {
        origin.truncate() / origin.w
    } else {
        Vec3::ZERO
    }
}

/// What the eyes see this frame, in some coordinates: each eye's view and projection, at most [`EYES`], and one frustum
/// holding all they see, or none when nothing is to be culled. Two on the headset; one on the desktop window and the
/// spectator.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Views {
    clips: [Mat4; EYES],
    count: usize,
    frustum: Option<Frustum>,
}

impl Default for Views {
    /// No eyes, culling nothing.
    fn default() -> Self {
        Self::unculled(&[])
    }
}

impl Views {
    /// The eyes `clips`, each a view and projection, and the frustum enclosing them all.
    ///
    /// # Panics
    ///
    /// If there are more than [`EYES`].
    pub fn new(clips: &[Mat4]) -> Self {
        Self {
            frustum: Some(Frustum::enclosing(clips)),
            ..Self::unculled(clips)
        }
    }

    /// The eyes `clips`, with nothing to be culled, as when culling is turned off to compare.
    ///
    /// # Panics
    ///
    /// If there are more than [`EYES`].
    pub fn unculled(clips: &[Mat4]) -> Self {
        assert!(clips.len() <= EYES, "{} eyes, of {EYES}", clips.len());
        let mut kept = [Mat4::IDENTITY; EYES];
        kept[..clips.len()].copy_from_slice(clips);
        Self {
            clips: kept,
            count: clips.len(),
            frustum: None,
        }
    }

    /// The same views of coordinates that `transform` carries into these, as a scenery's are carried into the space's:
    /// each eye's view and projection after `transform`, and the frustum holding the same.
    pub fn transformed(&self, transform: Mat4) -> Self {
        Self {
            clips: self.clips.map(|clip| clip * transform),
            count: self.count,
            frustum: self
                .frustum
                .as_ref()
                .map(|frustum| frustum.transformed(transform)),
        }
    }

    /// Each eye's view and projection.
    pub fn clips(&self) -> &[Mat4] {
        &self.clips[..self.count]
    }

    /// Where each eye is, in turn.
    pub fn eyes(&self) -> impl Iterator<Item = Vec3> + '_ {
        self.clips().iter().map(|clip| eye(*clip))
    }

    /// The frustum enclosing every eye's, outside which nothing is seen, or none when nothing is to be culled.
    pub fn frustum(&self) -> Option<&Frustum> {
        self.frustum.as_ref()
    }

    /// Whether anything of `bounds` may be seen: always, when nothing is to be culled.
    pub fn sees(&self, bounds: &Aabb) -> bool {
        self.frustum
            .as_ref()
            .is_none_or(|frustum| frustum.sees(bounds))
    }
}

#[cfg(test)]
pub(crate) mod tests;
