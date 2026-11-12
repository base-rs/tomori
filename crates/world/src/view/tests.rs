// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Views, where each eye is, and views carried into other coordinates.
use glam::Quat;

use super::*;

/// An eye at `position`, looking along −Z, 90° across and 2 km deep, as WebGPU's depth runs from 0 to 1.
pub(crate) fn looking(position: Vec3, heading: f32) -> Mat4 {
    glam::camera::rh::proj::directx::perspective(std::f32::consts::FRAC_PI_2, 1.0, 0.05, 2_000.0)
        * Mat4::from_rotation_translation(Quat::from_rotation_y(heading), position).inverse()
}

/// Two eyes 64 mm apart at `middle`, turned `heading` about the vertical.
pub(crate) fn both(middle: Vec3, heading: f32) -> [Mat4; 2] {
    let across = Quat::from_rotation_y(heading) * Vec3::X * 0.032;
    [
        looking(middle - across, heading),
        looking(middle + across, heading),
    ]
}

#[test]
fn views_hold_their_eyes_and_cull_what_neither_sees() {
    let eyes = both(Vec3::new(0.0, 1.6, 0.0), 0.0);
    let views = Views::new(&eyes);
    assert_eq!(views.clips(), &eyes);
    let positions: Vec<Vec3> = views.eyes().collect();
    assert!(positions[0].distance(Vec3::new(-0.032, 1.6, 0.0)) < 1e-4);
    assert!(positions[1].distance(Vec3::new(0.032, 1.6, 0.0)) < 1e-4);
    let ahead = Aabb::new(Vec3::new(-0.1, 1.5, -3.1), Vec3::new(0.1, 1.7, -2.9));
    let behind = Aabb::new(Vec3::new(-0.1, 1.5, 2.9), Vec3::new(0.1, 1.7, 3.1));
    assert!(views.sees(&ahead) && !views.sees(&behind));
    assert_eq!(views.frustum(), Some(&Frustum::enclosing(&eyes)));
    // Unculled, it sees everything; with no eyes, as by default, it is the same.
    let unculled = Views::unculled(&eyes);
    assert!(unculled.sees(&behind) && unculled.frustum().is_none());
    assert_eq!(Views::default(), Views::unculled(&[]));
    assert!(Views::default().clips().is_empty());
    // One view, as the desktop window's.
    let desktop = Views::new(&eyes[..1]);
    assert_eq!(desktop.clips().len(), 1);
    assert!(desktop.sees(&ahead) && !desktop.sees(&behind));
}

#[test]
#[should_panic(expected = "3 eyes, of 2")]
fn views_hold_no_more_than_a_headset_s_eyes() {
    Views::new(&[Mat4::IDENTITY; 3]);
}

#[test]
fn views_carried_into_a_scenery_s_coordinates_see_what_they_saw() {
    let eyes = both(Vec3::new(0.5, 1.6, 2.0), 0.6);
    let views = Views::new(&eyes);
    let carriage =
        Mat4::from_rotation_translation(Quat::from_rotation_y(1.2), Vec3::new(4.0, 0.0, -1.0));
    let carried = views.transformed(carriage);
    for (clip, carried) in views.clips().iter().zip(carried.clips()) {
        assert_eq!(*carried, *clip * carriage);
    }
    // A box ahead of the eyes, in the scenery's coordinates, is seen there, and one behind them is not.
    let ahead = carriage.inverse().transform_point3(
        Vec3::new(0.5, 1.6, 2.0) + Quat::from_rotation_y(0.6) * Vec3::new(0.0, 0.0, -5.0),
    );
    let behind = carriage.inverse().transform_point3(
        Vec3::new(0.5, 1.6, 2.0) + Quat::from_rotation_y(0.6) * Vec3::new(0.0, 0.0, 5.0),
    );
    assert!(carried.sees(&Aabb::new(ahead - 0.2, ahead + 0.2)));
    assert!(!carried.sees(&Aabb::new(behind - 0.2, behind + 0.2)));
    // Carried, unculled views still cull nothing.
    assert!(
        Views::unculled(&eyes)
            .transformed(carriage)
            .frustum()
            .is_none()
    );
}

#[test]
fn the_eye_is_where_every_ray_of_its_view_begins() {
    let position = Vec3::new(1.0, 1.6, 3.0);
    assert!(eye(looking(position, 0.4)).distance(position) < 1e-3);
    let infinite = glam::camera::rh::proj::directx::perspective_infinite(1.4, 1.0, 0.05)
        * Mat4::from_translation(position).inverse();
    assert!(eye(infinite).distance(position) < 1e-3);
    // A view with no such point, or none at all, is seen from the origin.
    assert_eq!(eye(Mat4::IDENTITY), Vec3::ZERO);
    assert_eq!(eye(Mat4::ZERO), Vec3::ZERO);
}
