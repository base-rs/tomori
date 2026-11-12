// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Where a body is, and how that is written on the wire: as whole numbers, to the millimetre and about a tenth of a
//! degree while it moves, and to a tenth of a millimetre and about a hundredth of a degree once it rests, so that what
//! the server simulates, keeps and sends, and what every client draws, agree to the bit.
use glam::{Mat4, Quat, Vec3};
use serde::{Deserialize, Serialize};

/// Where a body is and how it is turned, in the environment's own coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub position: Vec3,
    pub rotation: Quat,
}

impl Default for Placement {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Placement {
    /// At the origin, unturned.
    pub const IDENTITY: Self = Self {
        position: Vec3::ZERO,
        rotation: Quat::IDENTITY,
    };

    pub fn new(position: Vec3, rotation: Quat) -> Self {
        Self { position, rotation }
    }

    /// At `position`, unturned.
    pub fn at(position: Vec3) -> Self {
        Self::new(position, Quat::IDENTITY)
    }

    /// `local`, placed by this one: as a hand at this placement holds what it holds at the offset `local`.
    pub fn then(&self, local: &Self) -> Self {
        Self {
            position: self.position + self.rotation * local.position,
            rotation: (self.rotation * local.rotation).normalize(),
        }
    }

    /// The placement that [`Self::then`] undoes: so that `hand.inverse().then(&body)` is where `body` is held from
    /// `hand`.
    pub fn inverse(&self) -> Self {
        let rotation = self.rotation.inverse();
        Self {
            position: rotation * -self.position,
            rotation,
        }
    }

    /// `point`, in the placed body's own coordinates, where it is placed.
    pub fn transform_point(&self, point: Vec3) -> Vec3 {
        self.position + self.rotation * point
    }

    /// As a matrix, for drawing.
    pub fn matrix(&self) -> Mat4 {
        Mat4::from_rotation_translation(self.rotation, self.position)
    }

    /// Whether it is a placement at all: finite, with a rotation that can be made a unit one.
    pub fn valid(&self) -> bool {
        self.position.is_finite() && self.rotation.is_finite() && self.rotation.length() > 0.5
    }

    /// Written at `precision`.
    pub fn encoded(&self, precision: Precision) -> Encoded {
        let (place, turn) = precision.scales();
        let position = self.position * place;
        // One of the two quaternions for each rotation, so that the same rotation is always written the same way.
        let rotation = if self.rotation.w < 0.0 {
            -self.rotation
        } else {
            self.rotation
        };
        let rotation = rotation.normalize() * turn;
        let whole = |value: f32| value.round() as i32;
        Encoded([
            whole(position.x),
            whole(position.y),
            whole(position.z),
            whole(rotation.x),
            whole(rotation.y),
            whole(rotation.z),
            whole(rotation.w),
        ])
    }

    /// As it reads back once written at `precision`: what the server snaps a body that comes to rest to, so that it
    /// rests exactly where everyone is told it does.
    pub fn snapped(&self, precision: Precision) -> Self {
        self.encoded(precision).placement(precision)
    }
}

/// How finely a placement is written: [`Precision::Moving`], to the millimetre and a thousandth of each quaternion's
/// component, about a tenth of a degree, for the snapshots of what moves; [`Precision::Rest`], to a tenth of a
/// millimetre and a ten-thousandth, about a hundredth of a degree, for where a body comes to rest, is kept and is held.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Precision {
    Moving,
    Rest,
}

impl Precision {
    /// How many of each are a metre and a unit of a quaternion's component.
    fn scales(self) -> (f32, f32) {
        match self {
            Self::Moving => (1_000.0, 1_000.0),
            Self::Rest => (10_000.0, 10_000.0),
        }
    }
}

/// A placement written as seven whole numbers, its position's three and its rotation's four, at a [`Precision`] the
/// message it is in says.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Encoded(pub [i32; 7]);

impl Encoded {
    /// The placement it writes at `precision`. Its rotation is made a unit one in a fixed order of plain arithmetic,
    /// the same on every platform, so that a server and a headset read the same bits; one that writes no rotation, all
    /// zeros, is unturned.
    pub fn placement(&self, precision: Precision) -> Placement {
        let (place, turn) = precision.scales();
        let [x, y, z, i, j, k, w] = self.0.map(|value| value as f32);
        let (i, j, k, w) = (i / turn, j / turn, k / turn, w / turn);
        let length = (((i * i + j * j) + k * k) + w * w).sqrt();
        let rotation = if length > 0.0 {
            Quat::from_xyzw(i / length, j / length, k / length, w / length)
        } else {
            Quat::IDENTITY
        };
        Placement {
            position: Vec3::new(x / place, y / place, z / place),
            rotation,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_placement_held_at_an_offset_is_undone_by_the_inverse() {
        let hand = Placement::new(Vec3::new(1.0, 1.5, -2.0), Quat::from_rotation_y(0.7));
        let body = Placement::new(Vec3::new(1.1, 1.4, -2.2), Quat::from_rotation_x(0.3));
        let offset = hand.inverse().then(&body);
        let again = hand.then(&offset);
        assert!(again.position.distance(body.position) < 1e-5);
        assert!(again.rotation.angle_between(body.rotation) < 1e-3);
        assert!(
            hand.transform_point(Vec3::Z)
                .distance(hand.position + hand.rotation * Vec3::Z)
                < 1e-6
        );
        assert_eq!(Placement::default(), Placement::IDENTITY);
        assert_eq!(Placement::at(Vec3::X).position, Vec3::X);
        let matrix = body.matrix();
        assert!(
            matrix
                .transform_point3(Vec3::Y)
                .distance(body.transform_point(Vec3::Y))
                < 1e-5
        );
    }

    #[test]
    fn only_a_finite_placement_with_a_rotation_is_valid() {
        assert!(Placement::IDENTITY.valid());
        assert!(!Placement::at(Vec3::new(f32::NAN, 0.0, 0.0)).valid());
        assert!(!Placement::new(Vec3::ZERO, Quat::from_xyzw(0.0, 0.0, 0.0, f32::INFINITY)).valid());
        assert!(!Placement::new(Vec3::ZERO, Quat::from_xyzw(0.0, 0.0, 0.0, 0.1)).valid());
    }

    #[test]
    fn a_placement_reads_back_as_finely_as_it_was_written_and_the_same_every_time() {
        let placement = Placement::new(
            Vec3::new(12.34567, -0.58123, -5.43219),
            Quat::from_euler(glam::EulerRot::YXZ, 1.1, 0.2, -0.35),
        );
        for (precision, near, turned) in [
            (Precision::Moving, 0.0006, 0.004),
            (Precision::Rest, 0.00006, 0.0004),
        ] {
            let read = placement.encoded(precision).placement(precision);
            assert!(
                read.position.distance(placement.position) < near,
                "{precision:?}"
            );
            assert!(
                read.rotation.angle_between(placement.rotation) < turned,
                "{precision:?}"
            );
            // Snapped, it is what it reads back as, and snapping it again changes nothing, to the bit.
            let snapped = placement.snapped(precision);
            assert_eq!(snapped, read);
            assert_eq!(snapped.snapped(precision), snapped);
            assert_eq!(snapped.encoded(precision), placement.encoded(precision));
        }
        // Either quaternion of a rotation is written the same.
        let flipped = Placement::new(placement.position, -placement.rotation);
        assert_eq!(
            flipped.encoded(Precision::Rest),
            placement.encoded(Precision::Rest)
        );
        // Nothing written for the rotation reads as unturned.
        assert_eq!(
            Encoded([1000, 0, 0, 0, 0, 0, 0]).placement(Precision::Moving),
            Placement::at(Vec3::X)
        );
        assert_eq!(
            serde_json::to_string(&Encoded([1, 2, 3, 4, 5, 6, 7])).unwrap(),
            "[1,2,3,4,5,6,7]"
        );
    }
}
