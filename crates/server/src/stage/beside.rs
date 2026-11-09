// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Where a panel the agent floats beside someone stands: at a comfortable reading distance, a little below their eyes,
//! turned aside from straight ahead, to their right first and then their left, facing them, so that it is read by
//! turning the head a little and never hides what they look at. Several fan out about them rather than stacking, each
//! clear of the panels already floating there, and none stands inside the scenery or a wall, nor behind one. See
//! `design/projection.md`.
use tomori_environment::sightlines::Solid;
use tomori_protocol::{Pose, Wall};

use super::travel::{boxes, rotate};

/// How far from someone's eyes, horizontally, a panel floating beside them stands, in metres: within reach, and near
/// enough that a floating panel's text, at its 1.9 mm font pixel, reads comfortably, as `design/text.md` measures it.
pub const DISTANCE: f32 = 1.2;

/// How far below their eyes its middle stands, in metres, as a panel floating in front of them does.
pub const DROP: f32 = 0.15;

/// The bearings a panel beside someone is tried at, in degrees clockwise from the way they face, as seen from above:
/// 30° to their right, then 30° to their left, and then further out on each side, 40° apart, as far as a panel of
/// [`SIZE`] at [`DISTANCE`] takes, so that each stands beside the one before it rather than over it.
pub const BEARINGS: [f32; 6] = [30.0, -30.0, 70.0, -70.0, 110.0, -110.0];

/// The width and height a floating panel is taken to have, in metres, for keeping panels apart: a text panel at its
/// comfortable widest, 48 cells, about 85 cm, and about 17 rows. At [`DISTANCE`], one at the first bearing begins about
/// 10° from straight ahead, so that nothing ahead of them is hidden.
pub const SIZE: [f32; 2] = [0.85, 0.6];

/// How far from their eyes, horizontally, another floating panel is taken to be in the way, in metres.
const AROUND: f32 = 3.0;

/// How thick a wall stands about its slots, that a panel beside someone stays clear of, in metres.
const THICKNESS: f32 = 0.05;

/// The way someone whose head is at `head` faces, level, as a unit `(x, z)`: their forward, or, looking straight up or
/// down, the way the top of their head faces, or its back.
pub fn level_ahead(head: &Pose) -> (f32, f32) {
    let ahead = rotate(head.orientation, [0.0, 0.0, -1.0]);
    let level = |[x, _, z]: [f32; 3]| x.hypot(z);
    if level(ahead) > 1e-3 {
        return (ahead[0] / level(ahead), ahead[2] / level(ahead));
    }
    let top = rotate(head.orientation, [0.0, 1.0, 0.0]);
    let toward = if ahead[1] > 0.0 { -1.0 } else { 1.0 };
    let length = level(top).max(1e-6);
    (toward * top[0] / length, toward * top[2] / length)
}

/// Where a panel the agent floats beside someone whose head is at `head` stands: at the first of [`BEARINGS`] at which
/// it stands clear of `solids` and `walls`, nothing of them between it and their eyes, and of every panel `floating`
/// near them, as seen from their eyes; with every such spot taken, the first clear of the scenery and walls alone,
/// before what floats there; and with none clear, none, for the caller to float it in front of them.
pub fn spot(head: &Pose, walls: &[Wall], floating: &[Pose], solids: &[Solid]) -> Option<Pose> {
    let eyes = head.position;
    let (x, z) = level_ahead(head);
    let facing = x.atan2(-z);
    let mut blocking: Vec<Solid> = solids.to_vec();
    blocking.extend(walls.iter().flat_map(|wall| {
        boxes(wall).map(|(low, high)| {
            Solid::new(
                low.map(|value| value - THICKNESS),
                high.map(|value| value + THICKNESS),
            )
        })
    }));
    let clear: Vec<(f32, Pose)> = BEARINGS
        .iter()
        .map(|bearing| facing + bearing.to_radians())
        .map(|bearing| (bearing, at(eyes, bearing)))
        .filter(|(_, pose)| super::valid(pose).is_some() && unblocked(eyes, pose, &blocking))
        .collect();
    clear
        .iter()
        .find(|(bearing, pose)| {
            !floating
                .iter()
                .any(|other| overlaps(eyes, *bearing, pose, other))
        })
        .or_else(|| clear.first())
        .map(|(_, pose)| *pose)
}

/// The pose [`DISTANCE`] from `eyes` along `bearing`, in radians clockwise from −Z as seen from above, [`DROP`] below
/// them, upright and facing them.
fn at(eyes: [f32; 3], bearing: f32) -> Pose {
    let middle = [
        eyes[0] + DISTANCE * bearing.sin(),
        eyes[1] - DROP,
        eyes[2] - DISTANCE * bearing.cos(),
    ];
    Pose::upright_facing(middle, [eyes[0], middle[1], eyes[2]])
}

/// Whether a panel of [`SIZE`] at `pose` stands clear of `blocking`: no edge of it, nor either diagonal, passes
/// through any, nor does the line from `eyes` to its middle.
fn unblocked(eyes: [f32; 3], pose: &Pose, blocking: &[Solid]) -> bool {
    let right = rotate(pose.orientation, [1.0, 0.0, 0.0]);
    let corner = |across: f32, rise: f32| -> [f32; 3] {
        std::array::from_fn(|axis| {
            let up = if axis == 1 { 1.0 } else { 0.0 };
            pose.position[axis] + right[axis] * across * SIZE[0] / 2.0 + up * rise * SIZE[1] / 2.0
        })
    };
    let (a, b, c, d) = (
        corner(-1.0, -1.0),
        corner(1.0, -1.0),
        corner(-1.0, 1.0),
        corner(1.0, 1.0),
    );
    let lines = [
        (a, b),
        (b, d),
        (d, c),
        (c, a),
        (a, d),
        (b, c),
        (eyes, pose.position),
    ];
    !lines
        .into_iter()
        .any(|(from, to)| blocking.iter().any(|solid| solid.blocks(from, to)))
}

/// Whether a panel at `pose`, along `bearing` from `eyes`, would stand over `other`, a panel floating there, as seen
/// from them: their extents across, as angles about the eyes, and up and down, overlap, for one within [`AROUND`].
fn overlaps(eyes: [f32; 3], bearing: f32, pose: &Pose, other: &Pose) -> bool {
    let (dx, dz) = (other.position[0] - eyes[0], other.position[2] - eyes[2]);
    let distance = dx.hypot(dz);
    if distance > AROUND {
        return false;
    }
    let half = |distance: f32| (SIZE[0] / 2.0).atan2(distance.max(1e-3));
    let apart = (bearing - dx.atan2(-dz) + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU)
        - std::f32::consts::PI;
    apart.abs() < half(DISTANCE) + half(distance)
        && (other.position[1] - pose.position[1]).abs() < SIZE[1]
}

#[cfg(test)]
mod tests {
    use super::*;
    use tomori_protocol::WallKind;

    /// Someone at `eyes`, looking level `yaw` degrees clockwise from −Z: a head looks along its −Z, so its +Z faces
    /// the other way.
    fn head(eyes: [f32; 3], yaw: f32) -> Pose {
        let yaw = yaw.to_radians();
        Pose::at(eyes).facing_towards([eyes[0] - yaw.sin(), eyes[1], eyes[2] + yaw.cos()])
    }

    /// The bearing of `pose` from `eyes`, in degrees clockwise from −Z.
    fn bearing(eyes: [f32; 3], pose: &Pose) -> f32 {
        (pose.position[0] - eyes[0])
            .atan2(-(pose.position[2] - eyes[2]))
            .to_degrees()
    }

    fn faces(pose: &Pose, eyes: [f32; 3]) -> bool {
        let forward = rotate(pose.orientation, [0.0, 0.0, 1.0]);
        let toward = [eyes[0] - pose.position[0], 0.0, eyes[2] - pose.position[2]];
        let length = toward[0].hypot(toward[2]);
        (forward[0] * toward[0] + forward[2] * toward[2]) / length > 0.999
            && forward[1].abs() < 1e-3
    }

    #[test]
    fn a_panel_stands_beside_them_to_their_right_at_reading_distance_facing_them() {
        let eyes = [2.0, 1.6, 3.0];
        let pose = spot(&head(eyes, 0.0), &[], &[], &[]).expect("clear");
        let across = (pose.position[0] - eyes[0]).hypot(pose.position[2] - eyes[2]);
        assert!((across - DISTANCE).abs() < 1e-4, "{pose:?}");
        assert!((pose.position[1] - (eyes[1] - DROP)).abs() < 1e-5);
        assert!((bearing(eyes, &pose) - 30.0).abs() < 1e-3);
        assert!(faces(&pose, eyes));
        // Clear of straight ahead: its nearer edge is about 10° to the side.
        let edge = 30.0 - (SIZE[0] / 2.0).atan2(DISTANCE).to_degrees();
        assert!(edge > 9.0, "{edge}");
        // Turned, so is it: facing +X, beside them is towards +Z.
        let turned = spot(&head(eyes, 90.0), &[], &[], &[]).expect("clear");
        assert!((bearing(eyes, &turned) - 120.0).abs() < 1e-3);
        assert!(turned.position[2] > eyes[2]);
        // Looking straight down, beside them is beside the way the top of their head faces.
        let down = Pose::at(eyes).facing_towards([eyes[0], 9.0, eyes[2] + 1e-6]);
        let below = spot(&down, &[], &[], &[]).expect("clear");
        assert!((below.position[1] - (eyes[1] - DROP)).abs() < 1e-5);
        // Looking straight up, beside the way the back of their head faces.
        let up = Pose::at(eyes).facing_towards([eyes[0], -9.0, eyes[2] + 1e-6]);
        let above = spot(&up, &[], &[], &[]).expect("clear");
        let (x, z) = level_ahead(&up);
        assert!((x.hypot(z) - 1.0).abs() < 1e-4);
        assert!((above.position[1] - (eyes[1] - DROP)).abs() < 1e-5);
    }

    #[test]
    fn several_fan_out_right_then_left_then_further_out_none_over_another() {
        let eyes = [0.0, 1.6, 0.0];
        let head = head(eyes, 0.0);
        let mut floating = Vec::new();
        for expected in BEARINGS {
            let pose = spot(&head, &[], &floating, &[]).expect("clear");
            assert!(
                (bearing(eyes, &pose) - expected).abs() < 1e-3,
                "{expected}: {pose:?}"
            );
            for other in &floating {
                assert!(!overlaps(
                    eyes,
                    bearing(eyes, &pose).to_radians(),
                    &pose,
                    other
                ));
            }
            floating.push(pose);
        }
        // With every spot taken, the first again, before what floats there.
        let more = spot(&head, &[], &floating, &[]).expect("clear");
        assert!((bearing(eyes, &more) - 30.0).abs() < 1e-3);
        // A panel floating far away, or well above, is not in the way.
        let far = Pose::upright_facing([1.6, 1.45, -3.0], eyes);
        let high = Pose::upright_facing([0.6, 2.4, -1.04], eyes);
        let pose = spot(&head, &[], &[far, high], &[]).expect("clear");
        assert!((bearing(eyes, &pose) - 30.0).abs() < 1e-3);
        // One a participant left just to their right is: it goes to their left.
        let there = Pose::upright_facing([0.7, 1.5, -1.0], eyes);
        let pose = spot(&head, &[], &[there], &[]).expect("clear");
        assert!((bearing(eyes, &pose) + 30.0).abs() < 1e-3);
    }

    #[test]
    fn scenery_or_a_wall_to_their_right_sends_it_to_their_left_and_walled_in_it_has_none() {
        let eyes = [0.0, 1.6, 0.0];
        let head = head(eyes, 0.0);
        // A shed just to their right.
        let shed = Solid::new([0.3, 0.0, -2.0], [2.0, 2.5, -0.6]);
        let pose = spot(&head, &[], &[], &[shed]).expect("clear");
        assert!((bearing(eyes, &pose) + 30.0).abs() < 1e-3, "{pose:?}");
        // A wall someone added there, the same.
        let wall = Wall::new(
            "wall-1",
            WallKind::Added,
            Pose::upright_facing([0.6, 1.4, -1.04], eyes),
        );
        let pose = spot(&head, std::slice::from_ref(&wall), &[], &[]).expect("clear");
        assert!(bearing(eyes, &pose) < 0.0, "{pose:?}");
        // Something between their eyes and the spot, though not across it: it is not seen, so not there either.
        let post = Solid::new([0.25, 1.3, -0.6], [0.35, 1.6, -0.5]);
        let pose = spot(&head, &[], &[], &[post]).expect("clear");
        assert!((bearing(eyes, &pose) + 30.0).abs() < 1e-3, "{pose:?}");
        // Walled in on every side.
        let around = Solid::new([-5.0, -1.0, -5.0], [5.0, 3.0, 5.0]);
        assert_eq!(spot(&head, &[], &[], &[around]), None);
        // Beyond the space's reach, nowhere.
        assert_eq!(
            spot(&super::tests::head([30.0, 1.6, 0.0], 0.0), &[], &[], &[]),
            None
        );
    }
}
