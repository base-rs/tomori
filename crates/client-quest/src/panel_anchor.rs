// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Where the panel stands in a space: anchored in the world in front of the participant, and following them lazily, rather than fixed to their head. See `design/interaction.md`.
//!
//! The panel stays where it is while the participant looks around, so they can look past it at the swarm or a worker station and read it without it swimming against the world. Only when their head has turned more than [`TURN`] away from it, or moved more than [`MOVE`] from where it was placed, for longer than [`DWELL`], does it glide back in front of them over [`GLIDE`]. It is kept upright, turning only about the vertical axis, so looking down at it does not tip it.
//!
//! This is free of rendering code: it turns a head pose into the panel's pose each frame.

use glam::{Mat4, Quat, Vec3};

/// How far the head may turn away from the panel before it follows.
pub const TURN: f32 = std::f32::consts::FRAC_PI_4;
/// How far the head may move from where the panel was placed before it follows.
pub const MOVE: f32 = 0.5;
/// How long the head must stay turned or moved away before the panel follows, so that a glance never moves it.
pub const DWELL: f32 = 1.0;
/// How long the panel takes to glide back in front of the participant.
pub const GLIDE: f32 = 0.5;

/// An upright pose: a position and a heading about the vertical axis.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Placement {
    position: Vec3,
    /// Radians about +Y, zero facing -Z.
    yaw: f32,
}

impl Placement {
    /// The upright placement facing where `head` looks.
    fn of(head: Mat4) -> Self {
        let (_, rotation, position) = head.to_scale_rotation_translation();
        let forward = rotation * Vec3::NEG_Z;
        Self {
            position,
            yaw: f32::atan2(-forward.x, -forward.z),
        }
    }

    fn matrix(self) -> Mat4 {
        Mat4::from_rotation_translation(Quat::from_rotation_y(self.yaw), self.position)
    }

    fn lerp(self, to: Self, amount: f32) -> Self {
        Self {
            position: self.position.lerp(to.position, amount),
            yaw: self.yaw + wrap(to.yaw - self.yaw) * amount,
        }
    }
}

/// `angle` in (-π, π].
fn wrap(angle: f32) -> f32 {
    let tau = std::f32::consts::TAU;
    let wrapped = (angle + std::f32::consts::PI).rem_euclid(tau) - std::f32::consts::PI;
    if wrapped <= -std::f32::consts::PI {
        wrapped + tau
    } else {
        wrapped
    }
}

/// Smoothstep, so a glide eases in and out.
fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The panel's pose in a space, following the participant lazily.
#[derive(Clone, Debug, Default)]
pub struct PanelAnchor {
    placed: Option<Placement>,
    /// How long the head has been turned or moved away.
    away: f32,
    /// A glide under way: where it started and how far through it is, from 0 to 1.
    glide: Option<(Placement, f32)>,
}

impl PanelAnchor {
    /// Place the panel in front of the participant on the next update, as on entering a space.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// The pose the panel is drawn relative to, given the head pose and the time since the last update, in seconds. The panel is drawn below and in front of this pose, as it was in front of the head.
    pub fn update(&mut self, head: Mat4, dt: f32) -> Mat4 {
        let target = Placement::of(head);
        let placed = *self.placed.get_or_insert(target);
        if let Some((start, progress)) = &mut self.glide {
            *progress = (*progress + dt / GLIDE).min(1.0);
            let placement = start.lerp(target, ease(*progress));
            self.placed = Some(placement);
            if *progress >= 1.0 {
                self.glide = None;
                self.away = 0.0;
            }
            return placement.matrix();
        }
        let turned = wrap(target.yaw - placed.yaw).abs() > TURN;
        let moved = target.position.distance(placed.position) > MOVE;
        if turned || moved {
            self.away += dt;
            if self.away > DWELL {
                self.glide = Some((placed, 0.0));
            }
        } else {
            self.away = 0.0;
        }
        placed.matrix()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn head(yaw_degrees: f32, position: Vec3) -> Mat4 {
        Mat4::from_rotation_translation(Quat::from_rotation_y(yaw_degrees.to_radians()), position)
    }

    fn yaw(pose: Mat4) -> f32 {
        Placement::of(pose).yaw.to_degrees()
    }

    #[test]
    fn the_panel_stays_put_while_the_head_looks_around() {
        let mut anchor = PanelAnchor::default();
        let start = anchor.update(head(0.0, Vec3::new(0.0, 1.6, 0.0)), 0.0);
        // Looking 30 degrees aside for a long time, and nodding, does not move it.
        for _ in 0..200 {
            assert_eq!(
                anchor.update(head(30.0, Vec3::new(0.0, 1.6, 0.0)), 0.05),
                start
            );
        }
        let tilted =
            Mat4::from_rotation_translation(Quat::from_rotation_x(-0.6), Vec3::new(0.0, 1.6, 0.0));
        assert_eq!(anchor.update(tilted, 0.05), start);
    }

    #[test]
    fn a_glance_away_does_not_move_the_panel_but_turning_away_does_after_a_second() {
        let mut anchor = PanelAnchor::default();
        let origin = Vec3::new(0.0, 1.6, 0.0);
        let start = anchor.update(head(0.0, origin), 0.0);
        // A glance of under a second at the workers behind does not move it.
        for _ in 0..15 {
            assert_eq!(anchor.update(head(90.0, origin), 0.05), start);
        }
        assert_eq!(anchor.update(head(0.0, origin), 0.05), start);
        // Turning away for longer starts a glide that eases it round over half a second.
        for _ in 0..19 {
            assert_eq!(anchor.update(head(90.0, origin), 0.05), start);
        }
        let mut previous = 0.0;
        let mut frames = 0;
        loop {
            let current = yaw(anchor.update(head(90.0, origin), 0.05));
            assert!(current >= previous - 1e-3, "{current} after {previous}");
            previous = current;
            frames += 1;
            if (current - 90.0).abs() < 1e-3 {
                break;
            }
            assert!(frames <= 11, "glided for {frames} frames");
        }
        // And then stays.
        assert!((yaw(anchor.update(head(100.0, origin), 0.05)) - 90.0).abs() < 1e-3);
    }

    #[test]
    fn walking_away_brings_the_panel_along_and_reset_places_it_at_once() {
        let mut anchor = PanelAnchor::default();
        anchor.update(head(0.0, Vec3::new(0.0, 1.6, 0.0)), 0.0);
        let far = Vec3::new(2.0, 1.6, 0.0);
        for _ in 0..40 {
            anchor.update(head(0.0, far), 0.05);
        }
        let (_, _, position) = anchor
            .update(head(0.0, far), 0.05)
            .to_scale_rotation_translation();
        assert!(position.distance(far) < 1e-3);

        anchor.reset();
        let placed = anchor.update(head(180.0, Vec3::ZERO), 0.0);
        assert!((yaw(placed).abs() - 180.0).abs() < 1e-3);
    }

    #[test]
    fn headings_wrap_the_short_way_round() {
        assert!((wrap(3.5) - (3.5 - std::f32::consts::TAU)).abs() < 1e-5);
        let from = Placement {
            position: Vec3::ZERO,
            yaw: 170_f32.to_radians(),
        };
        let to = Placement {
            position: Vec3::ZERO,
            yaw: -170_f32.to_radians(),
        };
        let half = from.lerp(to, 0.5).yaw.to_degrees();
        assert!((wrap(half.to_radians()).to_degrees().abs() - 180.0).abs() < 1e-3);
    }
}
