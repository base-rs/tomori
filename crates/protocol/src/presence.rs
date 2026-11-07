// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Hand joint positions, and where the participant's room is, carried in a participant's presence update.
use serde::{Deserialize, Serialize};

/// The furthest a rig's translation may take a room from home along any axis, in metres; a rig beyond it is out of
/// bounds.
pub const RIG_REACH: f32 = 10_000.0;

/// Where a participant's room is in the space's scene, as their travel has taken it (see `design/locomotion.md`): a
/// translation, then a turn of `yaw` radians about the vertical, anticlockwise seen from above, so that a point in the
/// room is turned, then translated. Presence's poses are already in the scene, with the rig applied, so a receiver
/// that ignores it sees each participant where they travelled; one that reads it can tell their travel from their
/// walking. Sent compactly, as `[x, y, z, yaw]`, with every presence, home included as `[0, 0, 0, 0]`, since a rig
/// that is absent means only that the sender does not say, as an older client does not. A rig is in bounds when
/// every value is finite, each axis of its translation is within [`RIG_REACH`], and its yaw within a full turn
/// either way; one that is not, or cannot be read, is dropped as the presence is read, keeping the presence. More
/// may be added, so this is non-exhaustive: make one with [`Rig::new`]. See `design/spaces.md`.
///
/// ```
/// let rig = tomori_protocol::Rig::new([4.0, 3.0, -11.0], std::f32::consts::FRAC_PI_6);
/// assert!(rig.valid());
/// ```
///
/// A rig cannot be given its fields directly outside this crate:
///
/// ```compile_fail
/// let rig = tomori_protocol::Rig { translation: [0.0; 3], yaw: 0.0 };
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(from = "[f32; 4]", into = "[f32; 4]")]
#[non_exhaustive]
pub struct Rig {
    /// Where the room's origin is in the scene, its floor at the translation's height.
    pub translation: [f32; 3],
    /// How far the room is turned about the vertical, in radians, anticlockwise seen from above.
    pub yaw: f32,
}

impl Rig {
    /// A rig translating the room by `translation` after turning it by `yaw`.
    pub fn new(translation: [f32; 3], yaw: f32) -> Self {
        Self { translation, yaw }
    }

    /// Whether the rig is in bounds: finite, its translation within [`RIG_REACH`] along each axis, and its yaw within
    /// a full turn either way.
    pub fn valid(&self) -> bool {
        self.translation
            .iter()
            .all(|axis| axis.is_finite() && axis.abs() <= RIG_REACH)
            && self.yaw.is_finite()
            && self.yaw.abs() <= std::f32::consts::TAU
    }
}

impl From<[f32; 4]> for Rig {
    fn from([x, y, z, yaw]: [f32; 4]) -> Self {
        Self::new([x, y, z], yaw)
    }
}

impl From<Rig> for [f32; 4] {
    fn from(rig: Rig) -> Self {
        let [x, y, z] = rig.translation;
        [x, y, z, rig.yaw]
    }
}

/// A presence's rig as it is read: the rig when it can be read and is in bounds, and otherwise none, logged at debug
/// level, so that a rig that cannot be used never costs the poses it came with.
pub(crate) fn readable_rig<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Rig>, D::Error> {
    serde_json::Value::deserialize(deserializer).map(|field| match field {
        serde_json::Value::Null => None,
        field => match Rig::deserialize(field) {
            Ok(rig) if rig.valid() => Some(rig),
            Ok(_) => {
                log::debug!("Dropped a rig out of bounds.");
                None
            }
            Err(error) => {
                log::debug!("Dropped a rig that could not be read: {error}.");
                None
            }
        },
    })
}

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
    fn a_rig_is_in_bounds_only_when_finite_and_within_reach() {
        assert!(Rig::default().valid());
        assert!(Rig::new([RIG_REACH, -RIG_REACH, 3.0], std::f32::consts::TAU).valid());
        assert!(Rig::new([0.0; 3], -std::f32::consts::TAU).valid());
        for rig in [
            Rig::new([f32::NAN, 0.0, 0.0], 0.0),
            Rig::new([0.0, f32::INFINITY, 0.0], 0.0),
            Rig::new([0.0, 0.0, RIG_REACH * 2.0], 0.0),
            Rig::new([0.0; 3], f32::NAN),
            Rig::new([0.0; 3], 7.0),
            Rig::new([0.0; 3], -7.0),
        ] {
            assert!(!rig.valid(), "{rig:?}");
        }
        let rig = Rig::new([1.0, 2.0, 3.0], 0.5);
        assert_eq!(<[f32; 4]>::from(rig), [1.0, 2.0, 3.0, 0.5]);
        assert_eq!(Rig::from([1.0, 2.0, 3.0, 0.5]), rig);
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
