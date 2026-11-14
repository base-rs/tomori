// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Hand joint positions carried in a participant's presence update.
use serde::{Deserialize, Serialize};

/// The number of hand joints in the OpenXR EXT hand-joint order.
pub const JOINT_COUNT: usize = 26;

/// A single hand joint: its position in the space and the radius of the bone tip.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Joint {
    pub position: [f32; 3],
    pub radius: f32,
}

impl Joint {
    pub fn valid(&self) -> bool {
        self.position
            .iter()
            .all(|value| value.is_finite() && value.abs() < 10_000.0)
            && self.radius.is_finite()
            && (0.001..=0.1).contains(&self.radius)
    }
}

/// A tracked hand: up to [`JOINT_COUNT`] joints, each absent when the tracker
/// cannot see it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Hand {
    pub joints: [Option<Joint>; JOINT_COUNT],
}

impl Hand {
    pub fn valid(&self) -> bool {
        self.joints.iter().flatten().all(Joint::valid)
    }
}

/// The longest ray a presence report carries, in metres: well beyond what a space reaches.
pub const RAY_REACH: f32 = 100.0;

/// One of a participant's rays while it is drawn in full on their headset, so that the others draw it and the server
/// tells the space's agent what it points at: the aim pose it is drawn from, pointing along its −Z, how far it reaches
/// and whether it ends on something it meets. An idle stub, a hidden ray, the hand menu's and a teleport's aim are never
/// sent. See `design/people.md`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Ray {
    pub aim: crate::Pose,
    /// How far it reaches from its aim, in metres: to what it meets, or as far as an idle ray drawn in full reaches.
    pub length: f32,
    /// It ends on something it meets, where it is drawn ending in a dot.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub meets: bool,
}

impl Ray {
    /// Whether it can be drawn and resolved: its aim finite and within reach, with a rotation, and its length finite,
    /// more than nothing and at most [`RAY_REACH`].
    pub fn valid(&self) -> bool {
        let turn = self.aim.orientation;
        let length = turn.iter().map(|value| value * value).sum::<f32>().sqrt();
        self.aim
            .position
            .iter()
            .all(|value| value.is_finite() && value.abs() < 10_000.0)
            && length.is_finite()
            && length > 0.5
            && self.length.is_finite()
            && self.length > 0.0
            && self.length <= RAY_REACH
    }
}

/// A participant's rays drawn in full, left then right, as presence carries them.
pub type Pointing = [Option<Ray>; 2];

// OpenXR EXT hand-joint order: wrist followed by thumb and four fingers.
pub const BONES: [(usize, usize); 24] = [
    (1, 2),
    (2, 3),
    (3, 4),
    (4, 5),
    (1, 6),
    (6, 7),
    (7, 8),
    (8, 9),
    (9, 10),
    (1, 11),
    (11, 12),
    (12, 13),
    (13, 14),
    (14, 15),
    (1, 16),
    (16, 17),
    (17, 18),
    (18, 19),
    (19, 20),
    (1, 21),
    (21, 22),
    (22, 23),
    (23, 24),
    (24, 25),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_joint_geometry_and_standard_indices() {
        assert!(
            BONES
                .iter()
                .all(|(a, b)| *a < JOINT_COUNT && *b < JOINT_COUNT)
        );
        assert!(
            Joint {
                position: [1.0, 2.0, 3.0],
                radius: 0.01
            }
            .valid()
        );
        assert!(
            !Joint {
                position: [f32::NAN, 0.0, 0.0],
                radius: 0.01
            }
            .valid()
        );
        assert!(
            !Joint {
                position: [0.0; 3],
                radius: -1.0
            }
            .valid()
        );
    }

    #[test]
    fn a_ray_is_valid_only_aimed_somewhere_within_reach() {
        let ray = Ray {
            aim: crate::Pose::at([0.25, 1.25, -0.5]),
            length: 2.5,
            meets: true,
        };
        assert!(ray.valid());
        let changed = |change: fn(&mut Ray)| {
            let mut ray = ray;
            change(&mut ray);
            ray.valid()
        };
        assert!(!changed(|ray| ray.aim.position[1] = f32::NAN));
        assert!(!changed(|ray| ray.aim.position[0] = 20_000.0));
        assert!(!changed(|ray| ray.aim.orientation = [0.0; 4]));
        assert!(!changed(|ray| ray.aim.orientation[3] = f32::INFINITY));
        assert!(!changed(|ray| ray.length = 0.0));
        assert!(!changed(|ray| ray.length = f32::NAN));
        assert!(!changed(|ray| ray.length = RAY_REACH + 1.0));
        // Meeting nothing is said by leaving it out.
        let idle = Ray {
            meets: false,
            ..ray
        };
        let json = serde_json::to_value(idle).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"aim": {"position": [0.25, 1.25, -0.5], "orientation": [0.0, 0.0, 0.0, 1.0]}, "length": 2.5})
        );
        assert_eq!(serde_json::from_value::<Ray>(json).unwrap(), idle);
    }

    #[test]
    fn round_trips_through_json() {
        let hand = Hand {
            joints: {
                let mut joints = [None; JOINT_COUNT];
                joints[1] = Some(Joint {
                    position: [0.1, 0.2, 0.3],
                    radius: 0.01,
                });
                joints[2] = Some(Joint {
                    position: [0.15, 0.25, 0.35],
                    radius: 0.008,
                });
                joints
            },
        };
        assert!(hand.valid());
        assert!(Hand::default().valid());
        let mut invalid = hand;
        invalid.joints[2] = Some(Joint {
            position: [0.0; 3],
            radius: 1.0,
        });
        assert!(!invalid.valid());
        let json = serde_json::to_string(&hand).unwrap();
        let back: Hand = serde_json::from_str(&json).unwrap();
        assert_eq!(hand, back);
    }
}
