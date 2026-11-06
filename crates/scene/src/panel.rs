// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Flat panels in the world, such as the hand menu: a panel's plane and orientation, and the rectangles of its rows in it. Each row is laid out once, as an [`Area`], which the renderer draws its row within and which [`crate::pointing`] points at, by meeting the panel's plane and finding the rectangle that holds the point, so that what is pointed at is what is drawn at any tilt. See `design/hand-menu.md`.
use glam::{Vec2, Vec3};

use crate::pointing::Ray;

/// A panel's plane: its origin, and its right and up, of unit length and at right angles, so that it faces along `right × up`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Plane {
    pub origin: Vec3,
    pub right: Vec3,
    pub up: Vec3,
}

impl Plane {
    /// The plane through `origin` facing along `normal`, of unit length, with its right level and its up as near the world's up as the way it faces allows, as a label facing the viewer is drawn. Facing straight up or down, its right is +X.
    pub fn facing(origin: Vec3, normal: Vec3) -> Self {
        let right = Vec3::Y.cross(normal).try_normalize().unwrap_or(Vec3::X);
        Self {
            origin,
            right,
            up: normal.cross(right),
        }
    }

    /// The way the plane faces, of unit length.
    pub fn normal(&self) -> Vec3 {
        self.right.cross(self.up)
    }

    /// The point `at` metres right of and above the origin, in the plane.
    pub fn at(&self, at: Vec2) -> Vec3 {
        self.origin + self.right * at.x + self.up * at.y
    }

    /// Where `ray` meets the plane, from either side: how far along the ray, and the point there in the plane's coordinates, metres right of and above its origin. `None` when the ray runs along the plane or points away from it.
    pub fn meet(&self, ray: &Ray) -> Option<(f32, Vec2)> {
        let normal = self.normal();
        let closing = ray.direction.dot(normal);
        if closing.abs() < 1e-6 {
            return None;
        }
        let distance = (self.origin - ray.origin).dot(normal) / closing;
        if distance < 0.0 {
            return None;
        }
        let offset = ray.at(distance) - self.origin;
        Some((
            distance,
            Vec2::new(offset.dot(self.right), offset.dot(self.up)),
        ))
    }
}

/// A rectangle in a panel's plane, in metres. It holds the points from `min` up to, but not including, `max`, so that rectangles side by side share their edge and a point on it is in exactly one of them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub min: Vec2,
    pub max: Vec2,
}

impl Rect {
    pub fn contains(&self, point: Vec2) -> bool {
        point.cmpge(self.min).all() && point.cmplt(self.max).all()
    }

    pub fn centre(&self) -> Vec2 {
        (self.min + self.max) / 2.0
    }
}

/// A rectangle of a panel, placed in the world: the area a row of the panel is drawn within and pointed at by.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Area {
    pub plane: Plane,
    pub rect: Rect,
}

impl Area {
    /// How far along `ray` it meets this area, or `None` when it meets the panel's plane outside it, or not at all.
    pub fn intersect(&self, ray: &Ray) -> Option<f32> {
        let (distance, point) = self.plane.meet(ray)?;
        self.rect.contains(point).then_some(distance)
    }

    /// The middle of the area, in the world.
    pub fn centre(&self) -> Vec3 {
        self.plane.at(self.rect.centre())
    }

    /// Where `point`, in the world, is in the panel's plane, in its coordinates, ignoring how far it stands in front of or behind it.
    pub fn local(&self, point: Vec3) -> Vec2 {
        let offset = point - self.plane.origin;
        Vec2::new(offset.dot(self.plane.right), offset.dot(self.plane.up))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A plane at `origin` tilted `degrees` back from upright, facing +Z.
    fn tilted(origin: Vec3, degrees: f32) -> Plane {
        let (sin, cos) = degrees.to_radians().sin_cos();
        Plane::facing(origin, Vec3::new(0.0, sin, cos))
    }

    #[test]
    fn a_plane_faces_its_normal_with_its_right_level() {
        for degrees in [0.0, 30.0, 60.0, -45.0] {
            let plane = tilted(Vec3::new(0.1, 1.2, -0.4), degrees);
            let (sin, cos) = f32::to_radians(degrees).sin_cos();
            assert!(plane.normal().distance(Vec3::new(0.0, sin, cos)) < 1e-6);
            assert!(plane.right.distance(Vec3::X) < 1e-6);
            assert!(plane.up.dot(Vec3::Y) > 0.0, "{degrees}: up stays up");
            assert!(plane.right.dot(plane.up).abs() < 1e-6);
        }
        // Facing straight up, its right is +X and its up runs away from the world's +Z.
        let flat = Plane::facing(Vec3::ZERO, Vec3::Y);
        assert_eq!(flat.right, Vec3::X);
        assert_eq!(flat.up, -Vec3::Z);
        assert_eq!(flat.normal(), Vec3::Y);
    }

    #[test]
    fn a_ray_meets_a_plane_from_either_side_and_not_along_or_away_from_it() {
        let plane = tilted(Vec3::new(0.0, 1.0, -0.5), 30.0);
        let point = plane.at(Vec2::new(0.12, -0.05));
        for origin in [
            Vec3::new(0.0, 1.6, 0.0),
            Vec3::new(0.3, 0.9, -0.1),
            Vec3::new(-0.2, 0.4, -1.2),
        ] {
            let ray = Ray::new(origin, point - origin).unwrap();
            let (distance, at) = plane.meet(&ray).unwrap();
            assert!((distance - origin.distance(point)).abs() < 1e-5);
            assert!(at.distance(Vec2::new(0.12, -0.05)) < 1e-5, "{at}");
            // Turned around, it points away.
            let away = Ray::new(origin, origin - point).unwrap();
            assert_eq!(plane.meet(&away), None);
        }
        let along = Ray::new(plane.origin + plane.normal() * 0.1, plane.right).unwrap();
        assert_eq!(plane.meet(&along), None);
    }

    #[test]
    fn rectangles_side_by_side_share_their_edge_without_overlapping() {
        let upper = Rect {
            min: Vec2::new(0.0, 0.0),
            max: Vec2::new(0.4, 0.024),
        };
        let lower = Rect {
            min: Vec2::new(0.0, -0.024),
            max: Vec2::new(0.4, 0.0),
        };
        let edge = Vec2::new(0.2, 0.0);
        assert!(upper.contains(edge));
        assert!(!lower.contains(edge));
        assert!(upper.contains(Vec2::ZERO));
        assert!(!upper.contains(Vec2::new(0.4, 0.01)));
        assert!(!upper.contains(Vec2::new(-1e-6, 0.01)));
        assert_eq!(upper.centre(), Vec2::new(0.2, 0.012));
    }

    #[test]
    fn an_area_is_met_only_within_its_rectangle() {
        let area = Area {
            plane: tilted(Vec3::new(0.0, 1.0, -0.5), 60.0),
            rect: Rect {
                min: Vec2::new(-0.02, -0.012),
                max: Vec2::new(0.42, 0.012),
            },
        };
        let eye = Vec3::new(0.0, 1.6, 0.0);
        let at = |x: f32, y: f32| Ray::new(eye, area.plane.at(Vec2::new(x, y)) - eye).unwrap();
        assert!(area.intersect(&at(0.41, 0.0)).is_some());
        assert!(area.intersect(&at(0.43, 0.0)).is_none());
        assert!(area.intersect(&at(0.2, 0.013)).is_none());
        assert!(area.intersect(&at(0.2, -0.011)).is_some());
        let centre = area.centre();
        assert!(area.local(centre).distance(Vec2::new(0.2, 0.0)) < 1e-6);
        // In front of the plane or behind it, a point is where it stands over the plane.
        let raised = centre + area.plane.normal() * 0.01;
        assert!(area.local(raised).distance(Vec2::new(0.2, 0.0)) < 1e-6);
    }
}
