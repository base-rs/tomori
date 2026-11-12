// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! How near and how far every client's eyes see: the headset's, the desktop window's and the spectator's, with WebGPU's
//! depth from 0 at [`NEAR`] to 1 at [`FAR`], in a 32-bit float depth buffer. See `design/rendering.md`.
//!
//! The far plane is a plane, so it cuts what is off to the side nearer than what is straight ahead: at 50 m, as it was
//! before November 2026, scenery 45 to 60 m away was cut or not as the head turned. It now stands far beyond any
//! scenery, which the haze hides long before it. The near plane, not the far, decides how finely depth is kept: what is
//! `d` metres away is drawn at about `1 - NEAR / d`, whatever the far plane, so a farther one costs nearly nothing near.

use glam::Mat4;

/// How near the eyes anything is drawn, in metres.
pub const NEAR: f32 = 0.05;

/// How far from the eyes anything is drawn, in metres: far beyond any scenery, whose haze has hidden it entirely long
/// before, so that nothing is cut off as the head turns.
pub const FAR: f32 = 2_000.0;

/// A symmetric perspective of `fov_y` radians from top to bottom, `aspect` as wide as it is high, right-handed with −Z
/// forward and WebGPU's depth, from [`NEAR`] to [`FAR`], as the desktop window and the spectator see.
pub fn perspective(fov_y: f32, aspect: f32) -> Mat4 {
    glam::camera::rh::proj::directx::perspective(fov_y, aspect, NEAR, FAR)
}

/// The depth at which what is `distance` metres before the eyes, along their forward axis, is drawn, from 0 at
/// [`NEAR`] to 1 at [`FAR`].
pub fn at(distance: f32) -> f32 {
    FAR / (FAR - NEAR) * (1.0 - NEAR / distance)
}

#[cfg(test)]
mod tests {
    use glam::Vec3;

    use super::*;

    #[test]
    fn the_perspective_maps_the_near_and_far_planes_to_webgpu_s_depth() {
        let projection = perspective(1.0, 1.5);
        assert!(projection.project_point3(Vec3::NEG_Z * NEAR).z.abs() < 1e-6);
        assert!((projection.project_point3(Vec3::NEG_Z * FAR).z - 1.0).abs() < 1e-6);
        for distance in [0.3, 2.3, 50.0, 400.0] {
            let depth = projection.project_point3(Vec3::NEG_Z * distance).z;
            assert!((depth - at(distance)).abs() < 1e-6, "{distance}: {depth}");
        }
    }

    /// What is farther is drawn deeper, in the depth buffer's own 32-bit floats: surfaces a millimetre apart within
    /// arm's length and across the room, a centimetre apart at 50 m, and 10 cm apart at 150 m, beyond any scenery.
    #[test]
    fn depth_keeps_nearer_before_farther_finely_enough_from_near_to_far() {
        for (from, to, apart) in [
            (NEAR + 0.001, 3.0, 0.001),
            (3.0, 50.0, 0.01),
            (50.0, 150.0, 0.1),
        ] {
            let mut distance = from;
            while distance < to {
                let [near, far] = [distance, distance + apart].map(at);
                assert!(near < far, "{distance} m: {near} and {far}");
                assert!((0.0..1.0).contains(&near));
                distance += apart;
            }
        }
    }
}
