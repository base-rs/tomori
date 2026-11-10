// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Where a wall's slots stand, as the boxes the agent's spot and the beside placement keep clear of. See
//! `design/projection.md`.
use tomori_protocol::{Pose, Wall};

/// `vector` turned by the quaternion `rotation`.
pub(crate) fn rotate([x, y, z, w]: [f32; 4], vector: [f32; 3]) -> [f32; 3] {
    let length = (x * x + y * y + z * z + w * w).sqrt().max(f32::EPSILON);
    let (x, y, z, w) = (x / length, y / length, z / length, w / length);
    // v + 2w(q × v) + 2q × (q × v)
    let cross = |a: [f32; 3], b: [f32; 3]| {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };
    let q = [x, y, z];
    let t = cross(q, vector).map(|value| 2.0 * value);
    let u = cross(q, t);
    std::array::from_fn(|axis| vector[axis] + w * t[axis] + u[axis])
}

/// The middle and four corners of `slot`, on a wall standing at `wall`, as it is turned: bottom left, bottom right,
/// top left and top right.
fn corners(slot: &tomori_protocol::wall::SlotFrame, wall: &Pose) -> ([f32; 3], [[f32; 3]; 4]) {
    let at = slot.pose(wall);
    let right = rotate(at.orientation, [1.0, 0.0, 0.0]);
    let up = rotate(at.orientation, [0.0, 1.0, 0.0]);
    let [width, height] = slot.size;
    let corner = |across: f32, rise: f32| {
        std::array::from_fn(|axis| {
            at.position[axis] + right[axis] * across * width / 2.0 + up[axis] * rise * height / 2.0
        })
    };
    (
        at.position,
        [
            corner(-1.0, -1.0),
            corner(1.0, -1.0),
            corner(-1.0, 1.0),
            corner(1.0, 1.0),
        ],
    )
}

/// The box about each slot of `wall`, where it stands, tilted as it tilts, aligned with the space's axes.
pub(crate) fn boxes(wall: &Wall) -> impl Iterator<Item = ([f32; 3], [f32; 3])> + '_ {
    wall.slot_frames().into_iter().map(move |slot| {
        let (middle, corners) = corners(&slot, &wall.frame());
        corners
            .into_iter()
            .fold((middle, middle), |(low, high), corner| {
                (
                    std::array::from_fn(|axis| low[axis].min(corner[axis])),
                    std::array::from_fn(|axis| high[axis].max(corner[axis])),
                )
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tomori_protocol::{TOMORI_WALL, WallKind, WallShape};

    #[test]
    fn a_vector_turns_with_its_quaternion_and_a_wall_s_boxes_stand_about_its_slots_as_it_tilts() {
        // A quarter turn about the vertical takes −Z to −X.
        let quarter = Pose::at([0.0; 3])
            .facing_towards([1.0, 0.0, 0.0])
            .orientation;
        let turned = rotate(quarter, [0.0, 0.0, 1.0]);
        assert!(
            (turned[0] - 1.0).abs() < 1e-5 && turned[2].abs() < 1e-5,
            "{turned:?}"
        );
        let wall = Wall::new(TOMORI_WALL, WallKind::Tomori, Pose::at([0.0, 0.52, -2.3]))
            .with_shape(WallShape::Arc { radius: 2.3 });
        let upright: Vec<_> = boxes(&wall).collect();
        assert_eq!(upright.len(), wall.slots() as usize);
        // Each box holds its slot's middle; tilted, each is deeper.
        for (slot, (low, high)) in wall.slot_frames().iter().zip(&upright) {
            let middle = slot.pose(&wall.frame()).position;
            assert!(
                (0..3)
                    .all(|axis| low[axis] - 1e-4 <= middle[axis]
                        && middle[axis] <= high[axis] + 1e-4)
            );
        }
        let leaning = wall.clone().with_tilt(Some(45.0));
        assert!(boxes(&leaning).all(|(low, high)| high[2] - low[2] > 0.1));
    }
}
