// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The lights a world holds, each root's environment light, and which lights light each instance: [`assign`], the one
//! function that decides it, within the budgets of [`LIGHTS`] a root and [`LIGHTS_PER_INSTANCE`] an instance. See
//! `docs/proposals/rendering-architecture.md`.
use glam::Vec3;

use crate::Aabb;
use crate::world::{Key, Mobility, RootId};

/// The most lights a root lights its items by in a frame: of more that the views may see, those nearest the eyes.
pub const LIGHTS: usize = 64;

/// The most lights an instance is lit by: of more that reach it, the brightest on it.
pub const LIGHTS_PER_INSTANCE: usize = 8;

/// How much of its brightness on an instance, over the brightest light left out, a light kept of more than
/// [`LIGHTS_PER_INSTANCE`] fades in over, so that two lights passing each other in brightness swap without a flicker.
pub const FADE: f32 = 0.2;

/// How much further than its reach a light is taken to reach an instance's bounds, so that rounding never leaves out a
/// light that lights some of it: a centimetre, and a ten-thousandth of its reach.
const MARGIN: (f32, f32) = (0.01, 1e-4);

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

    /// Its colour at its intensity, as bright as it is at its centre.
    pub fn radiance(&self) -> Vec3 {
        self.color * self.intensity
    }

    /// How bright it is `distance` from its centre, by its falloff, as its luminance: none at its reach and beyond.
    pub fn brightness(&self, distance: f32) -> f32 {
        let reach = self.reach.max(0.0);
        if distance >= reach {
            return 0.0;
        }
        let fraction = distance / reach;
        let falloff = match self.falloff {
            Falloff::Stylised => (1.0 - fraction) * (1.0 - fraction),
            Falloff::Physical => {
                let window = 1.0 - fraction.powi(4);
                window * window / (1.0 + distance * distance)
            }
        };
        self.radiance()
            .dot(Vec3::new(0.2126, 0.7152, 0.0722))
            .max(0.0)
            * falloff
    }

    /// Whether it adds any light at all: some reach and some colour.
    pub fn gives_light(&self) -> bool {
        self.reach > 0.0 && self.radiance().max_element() > 0.0
    }
}

/// A light that lights an instance: its index among its root's lights this frame, [`crate::World::root_lights`], and
/// how much of its light it gives, 1 unless it is fading at [`LIGHTS_PER_INSTANCE`]'s cut.
#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(C)]
pub struct Lit {
    pub light: u32,
    pub weight: f32,
}

/// Append to `lit` the lights of `lights` that light an instance whose bounds are `bounds`, or that may be anywhere for
/// none, or by nothing for an empty box, in the order of `lights`, at most [`LIGHTS_PER_INSTANCE`]: each whose sphere of
/// reach meets its bounds, and, of more, the brightest at their centre, each falling off as if it reached as much
/// further as the bounds are large, so that every light that meets them ranks; those near the cut fade by [`FADE`] as
/// they near the brightest left out, so that no light's coming or going is seen, and any as bright as it give nothing
/// with it. One that may be anywhere is lit by the brightest lights, wherever they are, without fading. What is the same
/// each time gives the same lights, ties going to the first.
///
/// Every light the world lights its instances by is assigned here, so that another assignment, such as clustered
/// forward lighting's, changes neither the lights nor what environments submit.
pub fn assign(lights: &[Light], bounds: Option<&Aabb>, lit: &mut Vec<Lit>) {
    // An empty box is nowhere, and lit by nothing; one not all numbers may be anywhere.
    if bounds.is_some_and(Aabb::is_empty) {
        return;
    }
    let bounds = bounds.filter(|bounds| bounds.is_finite());
    // The brightest so far, one more than are kept, by brightness and then index, without allocating.
    let mut brightest = [(0.0f32, 0u32); LIGHTS_PER_INSTANCE + 1];
    let mut count = 0;
    for (index, light) in lights.iter().enumerate() {
        if !light.gives_light() {
            continue;
        }
        let brightness = match bounds {
            Some(bounds) => {
                let nearest = light.position.max(bounds.min).min(bounds.max);
                let distance = nearest.distance(light.position);
                if distance.is_nan() || distance >= light.reach * (1.0 + MARGIN.1) + MARGIN.0 {
                    continue;
                }
                let centre = (bounds.min + bounds.max) * 0.5;
                let grown = Light {
                    reach: light.reach + (bounds.max - centre).length() + MARGIN.0,
                    ..*light
                };
                grown
                    .brightness(centre.distance(light.position))
                    .max(f32::MIN_POSITIVE)
            }
            None => light.brightness(0.0),
        };
        let candidate = (brightness, index as u32);
        let mut at = count.min(LIGHTS_PER_INSTANCE);
        if count > LIGHTS_PER_INSTANCE && candidate.0 <= brightest[at].0 {
            continue;
        }
        while at > 0 && candidate.0 > brightest[at - 1].0 {
            brightest[at] = brightest[at - 1];
            at -= 1;
        }
        brightest[at] = candidate;
        count = (count + 1).min(LIGHTS_PER_INSTANCE + 1);
    }
    let kept = count.min(LIGHTS_PER_INSTANCE);
    let left_out =
        (count > LIGHTS_PER_INSTANCE && bounds.is_some()).then(|| brightest[LIGHTS_PER_INSTANCE].0);
    let start = lit.len();
    lit.extend(brightest[..kept].iter().map(|(brightness, light)| Lit {
        light: *light,
        weight: left_out.map_or(1.0, |left_out| {
            ((brightness - left_out) / (FADE * brightness)).clamp(0.0, 1.0)
        }),
    }));
    // In the order of the lights, so that each pixel adds them up as it would every light.
    lit[start..].sort_unstable_by_key(|lit| lit.light);
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

#[cfg(test)]
mod tests;
