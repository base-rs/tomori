// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The scene's haze, one function that every pipeline drawing in the world fades what it draws by, with the
//! environment's colour and density. See `design/rendering.md`.
use glam::Vec3;

/// The shared WGSL haze functions, `haze_amount`, `hazed` and `hazed_glow`, which every pipeline that draws in the world
/// calls, with its haze as a `vec4<f32>`, [`Haze::to_array`], in its own uniforms.
pub const WGSL: &str = include_str!("haze.wgsl");

/// `source`, a pipeline's shader, after the shared haze functions, [`WGSL`], which it calls.
pub fn shader(source: &str) -> String {
    format!("{WGSL}\n{source}")
}

/// An environment's haze: what is `d` metres from the eye fades into `color`, linear RGB, by `1 - exp(-density d²)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Haze {
    pub color: Vec3,
    pub density: f32,
}

impl Haze {
    /// No haze: nothing fades however far away it is.
    pub const NONE: Self = Self::new(Vec3::ZERO, 0.0);

    /// Haze of `color`, `density` thick.
    pub const fn new(color: Vec3, density: f32) -> Self {
        Self { color, density }
    }

    /// How far what is `distance` metres away has faded into the haze's colour, from 0 to 1, as `haze_amount` in
    /// [`WGSL`] says.
    pub fn amount(&self, distance: f32) -> f32 {
        1.0 - (-self.density * distance * distance).exp()
    }

    /// `color`, `distance` metres away, seen through the haze, as `hazed` in [`WGSL`] says.
    pub fn apply(&self, color: Vec3, distance: f32) -> Vec3 {
        color.lerp(self.color, self.amount(distance))
    }

    /// The haze as its shaders read it: its colour, and its density in the fourth component.
    pub fn to_array(&self) -> [f32; 4] {
        self.color.extend(self.density).to_array()
    }
}

impl Default for Haze {
    fn default() -> Self {
        Self::NONE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn haze_fades_with_distance_squared_into_its_colour() {
        let haze = Haze::new(Vec3::new(0.1, 0.2, 0.3), 0.01);
        assert_eq!(haze.amount(0.0), 0.0);
        assert!((haze.amount(10.0) - (1.0 - (-1.0f32).exp())).abs() < 1e-6);
        assert_eq!(haze.apply(Vec3::ONE, 0.0), Vec3::ONE);
        assert!(haze.apply(Vec3::ONE, 1e4).abs_diff_eq(haze.color, 1e-6));
        assert_eq!(haze.to_array(), [0.1, 0.2, 0.3, 0.01]);
        assert_eq!(Haze::default(), Haze::NONE);
        assert_eq!(Haze::NONE.amount(1e3), 0.0);
        assert!(shader("fn main() {}").starts_with(WGSL));
        assert!(shader("fn main() {}").ends_with("fn main() {}"));
    }
}
