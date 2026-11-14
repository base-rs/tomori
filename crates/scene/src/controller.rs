// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Controller and hand poses, and which index is each hand's.
use glam::{Mat4, Quat, Vec3};

pub const LEFT: usize = 0;
pub const RIGHT: usize = 1;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub position: [f32; 3],
    pub orientation: [f32; 4],
}

/// The same pose, as the protocol carries it in presence.
impl From<Pose> for tomori_protocol::Pose {
    fn from(pose: Pose) -> Self {
        Self {
            position: pose.position,
            orientation: pose.orientation,
        }
    }
}

impl Pose {
    pub fn valid(&self) -> bool {
        let q = Quat::from_array(self.orientation);
        self.position
            .iter()
            .all(|n| n.is_finite() && n.abs() < 10_000.0)
            && q.is_finite()
            && (0.5..=1.5).contains(&q.length())
    }

    pub fn transform(&self) -> Mat4 {
        Mat4::from_rotation_translation(
            Quat::from_array(self.orientation).normalize(),
            Vec3::from_array(self.position),
        )
    }

    pub fn from_transform(transform: Mat4) -> Option<Self> {
        if !transform.is_finite() {
            return None;
        }
        let (_, rotation, position) = transform.to_scale_rotation_translation();
        let pose = Self {
            position: position.to_array(),
            orientation: rotation.to_array(),
        };
        pose.valid().then_some(pose)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pose_round_trip_and_invalid_rotations() {
        let original =
            Mat4::from_rotation_translation(Quat::from_rotation_y(0.7), Vec3::new(1.0, 2.0, -3.0));
        let pose = Pose::from_transform(original).unwrap();
        assert!(pose.transform().abs_diff_eq(original, 1e-5));
        assert!(
            !Pose {
                position: [0.0; 3],
                orientation: [0.0; 4]
            }
            .valid()
        );
        assert!(
            !Pose {
                position: [f32::NAN, 0.0, 0.0],
                orientation: [0.0, 0.0, 0.0, 1.0]
            }
            .valid()
        );
    }
}
