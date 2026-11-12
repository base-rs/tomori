// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The lights a world holds, and each root's environment light. Which light reaches which item, and the budgets, come
//! later; see `docs/proposals/rendering-architecture.md`.
use glam::Vec3;

use crate::world::{Key, Mobility, RootId};

/// How a light fades with distance `d` within its reach `r`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Falloff {
    /// `(1 − d/r)²`, the stylised falloff the farmyard's and Sky's lamps are lit by.
    #[default]
    Stylised,
    /// The inverse square, windowed to nothing at its reach.
    Physical,
}

/// A light: where it is in its root's coordinates, how far it reaches, its colour, linear RGB, and intensity, its
/// falloff, the layer its baked light is kept in, and whether it is static.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Light {
    pub root: RootId,
    pub position: Vec3,
    pub reach: f32,
    pub color: Vec3,
    pub intensity: f32,
    pub falloff: Falloff,
    pub layer: u32,
    pub mobility: Mobility,
}

impl Light {
    /// A dynamic light in `root` at `position`, reaching `reach`, of `color` at intensity 1, stylised, in layer 0.
    pub fn new(root: RootId, position: Vec3, reach: f32, color: Vec3) -> Self {
        Self {
            root,
            position,
            reach,
            color,
            intensity: 1.0,
            falloff: Falloff::Stylised,
            layer: 0,
            mobility: Mobility::Dynamic,
        }
    }
}

/// A light kept in a world, by its slot and that slot's generation, so that one removed is never mistaken for one added
/// in its place.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LightId(pub(crate) Key);

/// The light about an environment, in its root's coordinates: the sun's or moon's direction, towards it, and colour,
/// the sky's and the ground's ambient, and the haze, which what is `d` from the eye fades into by
/// `1 − exp(−density d²)`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EnvironmentLight {
    pub sun_direction: Vec3,
    pub sun: Vec3,
    pub sky: Vec3,
    pub ground: Vec3,
    pub haze: Vec3,
    pub haze_density: f32,
}
