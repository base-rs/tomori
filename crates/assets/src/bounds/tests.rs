// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Boxes, frustums, one frustum for both eyes, where skinned and placed primitives may be drawn, and which primitives
//! are closed.
use std::sync::Arc;

use glam::{Quat, Vec2};

use super::*;
use crate::{Instance, Material, Mesh, Node, Pose, Transform};

/// An eye's view and projection, as the headset makes them: an asymmetric frustum of tangents `(left, right, down, up)`,
/// right-handed with −Z forward, WebGPU's depth from 0 to 1, between the headset's near and far planes, at `position`
/// facing `facing`.
pub(crate) fn eye(
    (left, right, down, up): (f32, f32, f32, f32),
    position: Vec3,
    facing: Quat,
) -> Mat4 {
    use tomori_scene::scene::headset::{Fov, View, view_projection};
    view_projection(&View {
        pose: tomori_scene::controller::Pose {
            position: position.to_array(),
            orientation: facing.to_array(),
        },
        fov: Fov {
            left: left.atan(),
            right: right.atan(),
            down: down.atan(),
            up: up.atan(),
        },
    })
}

/// A Quest 3's eyes, about: 52° out, 41° in, 40° up and 50° down, 64 mm apart, 1.6 m up, looking 10° down and turned
/// `heading` about the vertical.
pub(crate) fn eyes(heading: f32) -> [Mat4; 2] {
    let tan = |degrees: f32| degrees.to_radians().tan();
    let facing = Quat::from_rotation_y(heading) * Quat::from_rotation_x(-10f32.to_radians());
    let middle = Vec3::new(0.0, 1.6, 0.0);
    let across = facing * Vec3::X * 0.032;
    [
        eye(
            (-tan(52.0), tan(41.0), -tan(50.0), tan(40.0)),
            middle - across,
            facing,
        ),
        eye(
            (-tan(41.0), tan(52.0), -tan(50.0), tan(40.0)),
            middle + across,
            facing,
        ),
    ]
}

/// A point of a fixed, scattered sequence in `[0, 1)`.
fn scatter(index: u32) -> f32 {
    let mixed = index.wrapping_mul(2_654_435_761) ^ (index >> 7).wrapping_mul(40_503);
    (mixed % 100_000) as f32 / 100_000.0
}

/// The point at `ndc` in `clip`'s view, at depth `depth` from its near plane, 0, to its far plane, 1, found in double
/// precision, as single precision puts what is at a far plane 2 km away metres astray.
fn unproject(clip: Mat4, ndc: Vec2, depth: f32) -> Vec3 {
    let point = clip.as_dmat4().inverse() * ndc.extend(depth).extend(1.0).as_dvec4();
    (point.xyz() / point.w).as_vec3()
}

#[test]
fn a_box_holds_its_points_and_moves_with_them() {
    let points = [Vec3::new(1.0, -2.0, 0.5), Vec3::new(-1.0, 3.0, 0.0)];
    let bounds = Aabb::from_points(points);
    assert_eq!(
        bounds,
        Aabb::new(Vec3::new(-1.0, -2.0, 0.0), Vec3::new(1.0, 3.0, 0.5))
    );
    assert!(!bounds.is_empty() && bounds.is_finite());
    assert!(Aabb::from_points([]).is_empty());
    assert_eq!(Aabb::EMPTY.union(bounds), bounds);
    assert_eq!(Aabb::EMPTY.grown(1.0), Aabb::EMPTY);
    assert_eq!(Aabb::EMPTY.transformed(Mat4::IDENTITY), Aabb::EMPTY);
    assert_eq!(
        bounds.grown(0.5),
        Aabb::new(Vec3::new(-1.5, -2.5, -0.5), Vec3::new(1.5, 3.5, 1.0))
    );
    // Turned an eighth about the vertical and moved, a unit cube's box is √2 across and stands where it was moved.
    let cube = Aabb::new(Vec3::splat(-0.5), Vec3::splat(0.5));
    let turned = cube.transformed(Mat4::from_rotation_translation(
        Quat::from_rotation_y(std::f32::consts::FRAC_PI_4),
        Vec3::new(3.0, 0.0, 0.0),
    ));
    let half = std::f32::consts::FRAC_1_SQRT_2;
    assert!(turned.min.distance(Vec3::new(3.0 - half, -0.5, -half)) < 1e-5);
    assert!(turned.max.distance(Vec3::new(3.0 + half, 0.5, half)) < 1e-5);
    // Every corner of the cube, moved, lies within it.
    let skew = Mat4::from_cols_array(&[
        1.0, 0.3, 0.0, 0.0, -0.2, 2.0, 0.1, 0.0, 0.0, 0.4, -1.5, 0.0, 1.0, 2.0, 3.0, 1.0,
    ]);
    let moved = cube.transformed(skew);
    for corner in 0..8 {
        let at = Vec3::new(
            if corner & 1 == 0 { -0.5 } else { 0.5 },
            if corner & 2 == 0 { -0.5 } else { 0.5 },
            if corner & 4 == 0 { -0.5 } else { 0.5 },
        );
        let point = skew.transform_point3(at);
        assert!(point.cmpge(moved.min - 1e-5).all() && point.cmple(moved.max + 1e-5).all());
    }
    assert!(!Aabb::new(Vec3::NAN, Vec3::ONE).is_finite());
}

#[test]
fn a_frustum_sees_what_is_in_its_view_and_nothing_wholly_outside_it() {
    let [left, _] = eyes(0.0);
    let frustum = Frustum::new(left);
    // Ahead of the eye, inside, as far as the farthest scenery; behind it, beyond the far plane and far out to the side,
    // outside.
    let ahead = Vec3::new(0.0, 1.4, -3.0);
    assert!(frustum.holds(ahead));
    assert!(frustum.holds(Vec3::new(0.0, 1.6, -300.0)));
    assert!(!frustum.holds(Vec3::new(0.0, 1.6, 3.0)));
    assert!(!frustum.holds(Vec3::new(0.0, 1.6, -tomori_scene::depth::FAR * 1.1)));
    assert!(!frustum.holds(Vec3::new(-30.0, 1.6, -3.0)));
    let around = |middle: Vec3, half: f32| Aabb::new(middle - half, middle + half);
    assert!(frustum.sees(&around(ahead, 0.1)));
    assert!(!frustum.sees(&around(Vec3::new(0.0, 1.6, 3.0), 0.5)));
    assert!(!frustum.sees(&around(Vec3::new(-30.0, 1.6, -3.0), 1.0)));
    // A box reaching into the view from beside it is seen, though its middle is not.
    assert!(frustum.sees(&around(Vec3::new(-6.0, 1.4, -3.0), 4.0)));
    // Nothing is in an empty box, and a box that is not all numbers may be anywhere.
    assert!(!frustum.sees(&Aabb::EMPTY));
    assert!(frustum.sees(&Aabb::new(Vec3::NAN, Vec3::NAN)));
    // A matrix that is not all numbers sees everything, as does one whose far plane has no direction.
    let everywhere = Frustum::new(Mat4::from_cols_array(&[f32::NAN; 16]));
    assert!(everywhere.holds(Vec3::new(0.0, 0.0, 1e6)));
    let flat = Frustum::new(Mat4::from_cols(Vec4::X, Vec4::Y, Vec4::ZERO, Vec4::W));
    assert!(flat.holds(Vec3::new(0.0, 0.0, -1e6)) && flat.holds(Vec3::new(0.0, 0.0, 1e6)));
}

#[test]
fn both_eyes_frustum_holds_everything_either_eye_sees_and_culls_what_neither_does() {
    for heading in [0.0, 1.0, 2.5, -2.0] {
        let eyes = eyes(heading);
        let both = Frustum::enclosing(&eyes);
        let each = eyes.map(Frustum::new);
        // Points scattered through each eye's view, near its edges and corners as well as within it, are in both eyes'.
        for (index, clip) in eyes.iter().enumerate() {
            for sample in 0..4_000u32 {
                let at =
                    |offset: u32| scatter(sample * 7 + offset + index as u32 * 100_000) * 2.0 - 1.0;
                let ndc = Vec2::new(at(1), at(2)).clamp(Vec2::NEG_ONE, Vec2::ONE);
                let depth = (at(3) * 0.5 + 0.5).powi(8);
                let point = unproject(*clip, ndc, depth);
                assert!(
                    both.holds(point),
                    "heading {heading}: {point} from eye {index}"
                );
            }
            // And its very corners.
            for corner in [
                Vec2::NEG_ONE,
                Vec2::ONE,
                Vec2::new(-1.0, 1.0),
                Vec2::new(1.0, -1.0),
            ] {
                for depth in [0.0, 1.0] {
                    assert!(both.holds(unproject(*clip, corner, depth)));
                }
            }
        }
        // A box either eye sees, the combined frustum sees.
        for sample in 0..2_000u32 {
            let at = |offset: u32| scatter(sample * 5 + offset + 900_000);
            let middle = Vec3::new(at(1) * 80.0 - 40.0, at(2) * 20.0 - 5.0, at(3) * 80.0 - 40.0);
            let bounds = Aabb::new(middle, middle + Vec3::splat(at(4) * 3.0));
            if each.iter().any(|eye| eye.sees(&bounds)) {
                assert!(both.sees(&bounds), "heading {heading}: {bounds:?}");
            }
        }
        // What is behind both eyes, or far to either side of both, it culls.
        let facing = Quat::from_rotation_y(heading);
        let behind = Vec3::new(0.0, 1.6, 0.0) + facing * Vec3::new(0.0, 0.0, 2.0);
        assert!(!both.sees(&Aabb::new(behind - 0.3, behind + 0.3)));
        for side in [-1.0, 1.0] {
            let beside = Vec3::new(0.0, 1.6, 0.0) + facing * Vec3::new(side * 20.0, 0.0, -1.0);
            assert!(!both.sees(&Aabb::new(beside - 0.5, beside + 0.5)));
        }
    }
    // What either eye sees as far away as any scenery, 300 m, off to its sides and corners as straight ahead, both eyes'
    // frustum holds, whichever way they look, so that it is drawn rather than culled as the head turns.
    for heading in (0..12).map(|step| step as f32 * std::f32::consts::FRAC_PI_6) {
        let eyes = eyes(heading);
        let both = Frustum::enclosing(&eyes);
        for clip in &eyes {
            for sample in 0..200u32 {
                let at = |offset: u32| scatter(sample * 3 + offset + 500_000) * 1.98 - 0.99;
                // Along the ray through that point of the view, from the near plane.
                let ndc = Vec2::new(at(1), at(2));
                let near = unproject(*clip, ndc, 0.0);
                let point = near + (unproject(*clip, ndc, 0.5) - near).normalize() * 300.0;
                let depth = clip.project_point3(point).z;
                assert!(
                    both.holds(point) && (0.0..1.0).contains(&depth),
                    "heading {heading}: {point}, depth {depth}"
                );
            }
        }
    }
    // One view's frustum, enclosed alone, is that view's, a little loosened.
    let [left, _] = eyes(0.0);
    let alone = Frustum::enclosing(&[left]);
    let point = unproject(left, Vec2::new(0.3, -0.2), 0.5);
    assert!(alone.holds(point));
    assert!(!alone.holds(Vec3::new(0.0, 1.6, 3.0)));
    // No view, or one that cannot be inverted, holds everything.
    assert!(Frustum::enclosing(&[]).holds(Vec3::splat(1e6)));
    assert!(Frustum::enclosing(&[left, Mat4::ZERO]).holds(Vec3::splat(1e6)));
}

/// A skinned arm, as `pose`'s tests make it, placed at `transform`.
fn arm_at(transform: Mat4, time: f32) -> (Arc<Model>, Draw) {
    let model = Arc::new(crate::pose::tests::arm());
    let mut instance = Instance::new(model.clone());
    instance.transform = transform;
    instance.set_pose(Pose::sampled(&model, 0, time));
    let draw = instance.draws().remove(0);
    (model, draw)
}

#[test]
fn a_skinned_primitive_is_drawn_within_its_joints_boxes_however_it_is_posed() {
    let transform = Mat4::from_scale_rotation_translation(
        Vec3::splat(1.5),
        Quat::from_rotation_y(0.7),
        Vec3::new(2.0, 0.0, -4.0),
    );
    for step in 0..=10 {
        let (model, draw) = arm_at(transform, step as f32 / 10.0);
        let primitive = &model.meshes[0].primitives[0];
        let extent = Extent::of(primitive);
        let bounds = extent.placed(&draw).unwrap();
        let drawn = crate::deformed(primitive, draw.joints.as_ref().unwrap(), draw.world);
        for point in drawn {
            assert!(
                point.cmpge(bounds.min - 1e-4).all() && point.cmple(bounds.max + 1e-4).all(),
                "{point} outside {bounds:?} at step {step}"
            );
        }
        assert!(!mirrored(&draw));
    }
    // Bent, the hand points up, so the box reaches up a forearm's length; at rest it is the triangle's own box.
    let (model, draw) = arm_at(Mat4::IDENTITY, 1.0);
    let primitive = &model.meshes[0].primitives[0];
    let extent = Extent::of(primitive);
    assert!(extent.placed(&draw).unwrap().max.y >= 1.0 - 1e-5);
    assert_eq!(
        extent.local(),
        Aabb::new(Vec3::ZERO, Vec3::new(2.0, 0.0, 0.0))
    );
    // With fewer joints' matrices than the skin's vertices are weighted to, it may be anywhere.
    let mut short = draw.clone();
    short.joints.as_mut().unwrap().pop();
    assert_eq!(extent.placed(&short), None);
    // A skin whose weights do not sum to one, or that are negative, may put its vertices anywhere.
    let mut light = primitive.clone();
    light.weights[0] = [0.5, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
    assert_eq!(Extent::of(&light).placed(&draw), None);
    let mut negative = primitive.clone();
    negative.weights[0] = [1.5, -0.5, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
    assert_eq!(Extent::of(&negative).placed(&draw), None);
    // Drawn without its joints, as by a node with no skin, it is placed by its node, as the GPU places it.
    let mut rigid = draw.clone();
    rigid.joints = None;
    rigid.world = Mat4::from_translation(Vec3::Y);
    assert_eq!(
        extent.placed(&rigid),
        Some(Aabb::new(Vec3::Y, Vec3::new(2.0, 1.0, 0.0)))
    );
    // Mirrored, by a joint or by its node, it is wound the other way about.
    let (_, draw) = arm_at(Mat4::from_scale(Vec3::new(-1.0, 1.0, 1.0)), 0.5);
    assert!(mirrored(&draw));
}

#[test]
fn a_primitive_that_is_not_skinned_is_drawn_within_its_box_moved_by_its_node() {
    let primitive = Primitive::new(
        vec![Vec3::ZERO, Vec3::X, Vec3::new(0.0, 2.0, 1.0)],
        Vec::new(),
        vec![0, 1, 2],
    );
    let world =
        Mat4::from_rotation_translation(Quat::from_rotation_x(0.4), Vec3::new(0.0, 1.0, 0.0));
    let model = Model::built(
        vec![Node::new("one", Transform::IDENTITY).with_mesh(0)],
        vec![0],
        vec![Mesh::new("one", vec![primitive.clone()])],
        Vec::new(),
        &crate::Limits::default(),
    )
    .unwrap();
    let mut instance = Instance::new(Arc::new(model));
    instance.transform = world;
    let draw = instance.draws().remove(0);
    let bounds = Extent::of(&primitive).placed(&draw).unwrap();
    for point in &primitive.positions {
        let moved = world.transform_point3(*point);
        assert!(moved.cmpge(bounds.min - 1e-5).all() && moved.cmple(bounds.max + 1e-5).all());
    }
    assert!(!mirrored(&draw));
    let mut flipped = draw.clone();
    flipped.world = Mat4::from_scale(Vec3::new(1.0, 1.0, -1.0));
    assert!(mirrored(&flipped));
}

#[test]
fn only_a_material_that_is_cut_out_has_holes_to_see_through() {
    let mut cut = Material::new("leaf", Vec4::ONE);
    cut.masked = true;
    let model = Model {
        materials: vec![Material::new("wood", Vec4::ONE), cut],
        ..Model::default()
    };
    assert!(opaque(&model, None));
    assert!(opaque(&model, Some(0)));
    assert!(!opaque(&model, Some(1)));
}

/// A box from `low` to `high`, its six faces each of two triangles wound counterclockwise seen from outside, with
/// shared corners, or, `faceted`, a corner of its own for each face.
pub(crate) fn cuboid(low: Vec3, high: Vec3, faceted: bool) -> Primitive {
    let corner = |index: usize| {
        Vec3::new(
            if index & 1 == 0 { low.x } else { high.x },
            if index & 2 == 0 { low.y } else { high.y },
            if index & 4 == 0 { low.z } else { high.z },
        )
    };
    // Each face's corners, counterclockwise from outside.
    let faces = [
        [0, 2, 3, 1], // −Z
        [4, 5, 7, 6], // +Z
        [0, 4, 6, 2], // −X
        [1, 3, 7, 5], // +X
        [0, 1, 5, 4], // −Y
        [2, 6, 7, 3], // +Y
    ];
    let mut positions = Vec::new();
    let mut indices = Vec::new();
    for face in faces {
        let corners: [u32; 4] = if faceted {
            let start = positions.len() as u32;
            positions.extend(face.map(corner));
            [start, start + 1, start + 2, start + 3]
        } else {
            face.map(|index| index as u32)
        };
        indices.extend([
            corners[0], corners[1], corners[2], corners[0], corners[2], corners[3],
        ]);
    }
    if !faceted {
        positions = (0..8).map(corner).collect();
    }
    Primitive::new(positions, Vec::new(), indices)
}

/// `primitive` with `other`'s triangles after its own.
fn joined(mut primitive: Primitive, other: &Primitive) -> Primitive {
    let offset = primitive.positions.len() as u32;
    primitive.positions.extend(&other.positions);
    primitive
        .indices
        .extend(other.indices.iter().map(|index| index + offset));
    primitive
}

/// `primitive` with every triangle wound the other way about.
fn inside_out(mut primitive: Primitive) -> Primitive {
    for triangle in primitive.indices.as_chunks_mut::<3>().0 {
        triangle.swap(1, 2);
    }
    primitive
}

#[test]
fn a_primitive_is_closed_only_when_every_part_of_it_encloses_its_volume_facing_out() {
    let unit = cuboid(Vec3::ZERO, Vec3::ONE, false);
    assert!(unit.closed());
    assert!(cuboid(Vec3::ZERO, Vec3::ONE, true).closed());
    // Several boxes, and a small one far from the origin, are closed.
    let far = cuboid(
        Vec3::new(400.0, 3.0, -250.0),
        Vec3::new(400.02, 3.01, -249.99),
        true,
    );
    assert!(far.closed());
    assert!(joined(unit.clone(), &far).closed());
    // Inside out, as a room seen from within it, or with a part inside out, it is not.
    assert!(!inside_out(unit.clone()).closed());
    assert!(!joined(unit.clone(), &inside_out(far.clone())).closed());
    // Nor with a face missing, as an open box, nor with a triangle twice, nor one with two corners at one point.
    let mut open = unit.clone();
    open.indices.truncate(30);
    assert!(!open.closed());
    let mut twice = unit.clone();
    twice.indices.extend_from_slice(&unit.indices[..3]);
    assert!(!twice.closed());
    let mut pinched = unit.clone();
    pinched.indices.extend([0, 0, 1, 1, 0, 0]);
    assert!(!pinched.closed());
    // A card drawn on both sides, as two layers wound each way, has no volume, and is not closed.
    let quad = Primitive::new(
        vec![Vec3::ZERO, Vec3::X, Vec3::new(1.0, 1.0, 0.0), Vec3::Y],
        Vec::new(),
        vec![0, 1, 2, 0, 2, 3, 0, 3, 1, 1, 3, 2],
    );
    assert!(!quad.closed());
    assert!(!Primitive::default().closed());
    // Of a primitive of a closed box and an open one, the closed box's triangles alone are closed.
    let both = joined(far.clone(), &open);
    let closed = both.closed_triangles();
    assert_eq!(closed.len(), 22);
    assert!(closed[..12].iter().all(|closed| *closed));
    assert!(closed[12..].iter().all(|closed| !*closed));
    assert!(!both.closed());
    // A box with a card through it, its edges shared three ways, is open where they are shared.
    let mut crossed = unit.clone();
    crossed.indices.extend([0, 3, 4]);
    assert!(crossed.closed_triangles().iter().all(|closed| !*closed));
}

#[test]
fn tallies_add_up() {
    let mut tally = Tally {
        drawn: 1,
        culled: 2,
        draws: 3,
        triangles: 4,
    };
    tally += tally;
    assert_eq!(
        tally + Tally::default(),
        Tally {
            drawn: 2,
            culled: 4,
            draws: 6,
            triangles: 8
        }
    );
}
