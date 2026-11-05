// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Pointing at objects in the world with a ray, and choosing them with a trigger. See `design/arrival.md`.
//!
//! This has no rendering, windowing or XR code, so the headset's controller and the desktop's mouse share it: each turns its device into a [`Ray`] and a trigger value, and draws what [`Pointing`] reports.
use glam::{Quat, Vec3};

/// How far the ray reaches when it meets nothing.
pub const RAY_LENGTH: f32 = 3.0;
/// How far the ray reaches in a space when it meets no worker: far enough to show where it points, but short of the workers, so that an idle ray does not streak through the space. It reaches a worker once it meets one.
pub const SPACE_RAY_LENGTH: f32 = 1.0;

/// The pointing ray, from the controller to where it ends.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Beam {
    pub origin: Vec3,
    pub end: Vec3,
    /// Whether it ends on something it points at, which draws it highlighted.
    pub hit: bool,
}

/// A pull starts once the trigger passes this, and attracts the swarm when it does not choose.
const PULL: f32 = 0.2;
/// A pull that began on a target chooses it once the trigger passes this.
const PRESS: f32 = 0.55;
/// A pull ends once the trigger falls below this, so a trigger resting near [`PULL`] does not flicker.
const RELEASE: f32 = 0.1;

/// A half-line in scene coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ray {
    pub origin: Vec3,
    /// A unit vector.
    pub direction: Vec3,
}

impl Ray {
    /// A ray from `origin` towards `direction`, or `None` when either is not finite or the direction has no length.
    pub fn new(origin: Vec3, direction: Vec3) -> Option<Self> {
        let direction = direction.try_normalize()?;
        origin.is_finite().then_some(Self { origin, direction })
    }

    /// The ray along a pose's forward axis, `-Z`, as for an OpenXR aim pose.
    pub fn from_pose(position: Vec3, orientation: Quat) -> Option<Self> {
        orientation
            .is_finite()
            .then(|| Self::new(position, orientation.normalize() * -Vec3::Z))
            .flatten()
    }

    pub fn at(&self, distance: f32) -> Vec3 {
        self.origin + self.direction * distance
    }
}

/// A simple shape to point at, in scene coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    Sphere {
        center: Vec3,
        radius: f32,
    },
    /// An axis-aligned box.
    Box {
        center: Vec3,
        half_extents: Vec3,
    },
    /// A rectangle of a flat panel, such as a row of the hand menu, met in the panel's own plane and orientation.
    Panel(crate::panel::Area),
}

impl Shape {
    /// The distance along `ray` to where it enters this shape, zero when it starts inside, or `None` when it misses.
    pub fn intersect(&self, ray: &Ray) -> Option<f32> {
        match *self {
            Shape::Sphere { center, radius } => {
                let offset = ray.origin - center;
                let b = offset.dot(ray.direction);
                let c = offset.length_squared() - radius * radius;
                if c <= 0.0 {
                    return Some(0.0);
                }
                let discriminant = b * b - c;
                if b > 0.0 || discriminant < 0.0 {
                    return None;
                }
                Some(-b - discriminant.sqrt())
            }
            Shape::Box {
                center,
                half_extents,
            } => {
                let minimum = center - half_extents;
                let maximum = center + half_extents;
                let mut near = 0.0_f32;
                let mut far = f32::INFINITY;
                for axis in 0..3 {
                    let (origin, direction) = (ray.origin[axis], ray.direction[axis]);
                    if direction.abs() < f32::EPSILON {
                        if origin < minimum[axis] || origin > maximum[axis] {
                            return None;
                        }
                        continue;
                    }
                    let a = (minimum[axis] - origin) / direction;
                    let b = (maximum[axis] - origin) / direction;
                    near = near.max(a.min(b));
                    far = far.min(a.max(b));
                    if near > far {
                        return None;
                    }
                }
                Some(near)
            }
            Shape::Panel(area) => area.intersect(ray),
        }
    }

    /// The middle of the shape, in scene coordinates.
    pub fn centre(&self) -> Vec3 {
        match *self {
            Shape::Sphere { center, .. } | Shape::Box { center, .. } => center,
            Shape::Panel(area) => area.centre(),
        }
    }
}

/// Something that can be pointed at, named by `key`.
#[derive(Clone, Debug, PartialEq)]
pub struct Target<K> {
    pub key: K,
    pub shape: Shape,
}

/// Where a ray meets a target.
#[derive(Clone, Debug, PartialEq)]
pub struct Hit<K> {
    pub key: K,
    pub distance: f32,
    pub point: Vec3,
}

/// The nearest target `ray` meets, if any. The first of equally near targets wins, so the result is stable.
pub fn nearest<'a, K: Clone + 'a>(
    ray: &Ray,
    targets: impl IntoIterator<Item = &'a Target<K>>,
) -> Option<Hit<K>> {
    let mut nearest: Option<Hit<K>> = None;
    for target in targets {
        if let Some(distance) = target.shape.intersect(ray)
            && nearest.as_ref().is_none_or(|hit| distance < hit.distance)
        {
            nearest = Some(Hit {
                key: target.key.clone(),
                distance,
                point: ray.at(distance),
            });
        }
    }
    nearest
}

/// Who owns the current trigger pull.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Pull {
    #[default]
    Released,
    /// The pull began away from every target, so it attracts the swarm until released, even across targets.
    Attracting,
    /// The pull began on a target, so it chooses rather than attracts; `fired` once it has chosen.
    Choosing { fired: bool },
}

/// One frame's result of pointing.
#[derive(Clone, Debug, PartialEq)]
pub struct Pointing<K> {
    /// The ray, when the device is tracked.
    pub ray: Option<Ray>,
    /// The target the ray meets, which is highlighted.
    pub hit: Option<Hit<K>>,
    /// The target chosen this frame, on the press of the trigger and never while it is held.
    pub chosen: Option<K>,
    /// Whether the trigger should attract the swarm instead: it is pulled, and the pull did not begin on a target.
    pub attracting: bool,
}

impl<K> Pointing<K> {
    /// Where to draw the end of the ray: the hit, or `length` along it.
    pub fn endpoint(&self, length: f32) -> Option<Vec3> {
        let ray = self.ray?;
        Some(
            self.hit
                .as_ref()
                .map_or_else(|| ray.at(length), |hit| hit.point),
        )
    }
}

/// Pointing and the trigger across frames: the highlighted target and who owns the pull.
///
/// A pull that begins while the ray is on a target chooses it once the trigger is pressed far enough, and only once until the trigger is released. A pull that begins anywhere else attracts the swarm, as the trigger did before pointing, and never chooses.
#[derive(Debug)]
pub struct Pointer<K> {
    pull: Pull,
    hovered: Option<K>,
}

impl<K> Default for Pointer<K> {
    fn default() -> Self {
        Self {
            pull: Pull::Released,
            hovered: None,
        }
    }
}

impl<K: Clone + PartialEq> Pointer<K> {
    /// `ray` is `None` while the device is untracked, and `trigger` is `None` while its trigger is inactive, as without focus; either ends what the pointer was doing.
    pub fn update<'a>(
        &mut self,
        ray: Option<Ray>,
        trigger: Option<f32>,
        targets: impl IntoIterator<Item = &'a Target<K>>,
    ) -> Pointing<K>
    where
        K: 'a,
    {
        let hit = ray.as_ref().and_then(|ray| nearest(ray, targets));
        self.hovered = hit.as_ref().map(|hit| hit.key.clone());
        let trigger = trigger.filter(|value| value.is_finite()).unwrap_or(0.0);
        let mut chosen = None;
        self.pull = match self.pull {
            _ if trigger < RELEASE => Pull::Released,
            Pull::Released if trigger <= PULL => Pull::Released,
            Pull::Released if self.hovered.is_some() => Pull::Choosing { fired: false },
            Pull::Released => Pull::Attracting,
            pull => pull,
        };
        if self.pull == (Pull::Choosing { fired: false }) && trigger >= PRESS {
            self.pull = Pull::Choosing { fired: true };
            chosen = self.hovered.clone();
        }
        Pointing {
            ray,
            hit,
            chosen,
            attracting: self.pull == Pull::Attracting,
        }
    }

    /// The highlighted target, as of the last update.
    #[cfg(test)]
    pub fn hovered(&self) -> Option<&K> {
        self.hovered.as_ref()
    }

    /// Forget the pull and the highlight, as when focus is lost.
    pub fn release(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sphere(key: &'static str, x: f32, z: f32) -> Target<&'static str> {
        Target {
            key,
            shape: Shape::Sphere {
                center: Vec3::new(x, 1.0, z),
                radius: 0.1,
            },
        }
    }

    fn forward() -> Ray {
        Ray::new(Vec3::new(0.0, 1.0, 0.0), -Vec3::Z).unwrap()
    }

    #[test]
    fn a_ray_needs_a_finite_origin_and_a_direction() {
        assert!(Ray::new(Vec3::ZERO, Vec3::ZERO).is_none());
        assert!(Ray::new(Vec3::NAN, Vec3::X).is_none());
        assert!(Ray::new(Vec3::ZERO, Vec3::new(f32::INFINITY, 0.0, 0.0)).is_none());
        let ray = Ray::new(Vec3::ZERO, Vec3::new(0.0, 0.0, -2.0)).unwrap();
        assert_eq!(ray.direction, -Vec3::Z);
        assert_eq!(ray.at(3.0), Vec3::new(0.0, 0.0, -3.0));
    }

    #[test]
    fn a_pose_points_along_its_negative_z_axis() {
        let ray =
            Ray::from_pose(Vec3::Y, Quat::from_rotation_y(std::f32::consts::FRAC_PI_2)).unwrap();
        assert!(ray.direction.distance(-Vec3::X) < 1e-5);
        assert_eq!(ray.origin, Vec3::Y);
        assert!(Ray::from_pose(Vec3::ZERO, Quat::from_xyzw(f32::NAN, 0.0, 0.0, 1.0)).is_none());
        // An unnormalised orientation from a runtime still points the same way.
        let scaled = Quat::from_xyzw(0.0, 0.0, 0.0, 2.0);
        assert!(
            Ray::from_pose(Vec3::ZERO, scaled)
                .unwrap()
                .direction
                .distance(-Vec3::Z)
                < 1e-5
        );
    }

    #[test]
    fn spheres_are_hit_in_front_and_missed_behind_or_beside() {
        let ray = forward();
        let ahead = sphere("a", 0.0, -2.0).shape;
        assert!((ahead.intersect(&ray).unwrap() - 1.9).abs() < 1e-5);
        assert_eq!(sphere("b", 0.0, 2.0).shape.intersect(&ray), None);
        assert_eq!(sphere("c", 0.5, -2.0).shape.intersect(&ray), None);
        // Grazing the edge still counts.
        assert!(sphere("d", 0.0999, -2.0).shape.intersect(&ray).is_some());
        // Starting inside hits at once.
        assert_eq!(sphere("e", 0.0, -0.05).shape.intersect(&ray), Some(0.0));
    }

    #[test]
    fn boxes_are_hit_on_their_faces_including_along_an_axis() {
        let shape = Shape::Box {
            center: Vec3::new(0.0, 1.0, -2.0),
            half_extents: Vec3::splat(0.2),
        };
        assert!((shape.intersect(&forward()).unwrap() - 1.8).abs() < 1e-5);
        // Parallel to the faces, outside them.
        let beside = Ray::new(Vec3::new(0.5, 1.0, 0.0), -Vec3::Z).unwrap();
        assert_eq!(shape.intersect(&beside), None);
        // Diagonal through a corner region.
        let diagonal = Ray::new(Vec3::new(1.0, 1.0, -1.0), Vec3::new(-1.0, 0.0, -1.0)).unwrap();
        assert!(shape.intersect(&diagonal).is_some());
        let away = Ray::new(Vec3::new(0.0, 1.0, 0.0), Vec3::Z).unwrap();
        assert_eq!(shape.intersect(&away), None);
        let inside = Ray::new(Vec3::new(0.0, 1.0, -2.0), Vec3::X).unwrap();
        assert_eq!(shape.intersect(&inside), Some(0.0));
        let skew = Ray::new(Vec3::new(0.0, 1.5, 0.0), Vec3::new(1.0, 0.0, -1.0)).unwrap();
        assert_eq!(shape.intersect(&skew), None);
        assert_eq!(shape.centre(), Vec3::new(0.0, 1.0, -2.0));
        assert_eq!(
            sphere("a", 0.5, -2.0).shape.centre(),
            Vec3::new(0.5, 1.0, -2.0)
        );
    }

    #[test]
    fn a_panel_s_rectangle_is_met_in_its_plane_and_missed_beside_or_behind_it() {
        use crate::panel::{Area, Plane, Rect};
        use glam::Vec2;
        // A rectangle 40 cm wide and 2.4 cm tall, tilted 60° back, below and ahead of the eye.
        let (sin, cos) = 60f32.to_radians().sin_cos();
        let shape = Shape::Panel(Area {
            plane: Plane::facing(Vec3::new(-0.2, 1.0, -0.4), Vec3::new(0.0, sin, cos)),
            rect: Rect {
                min: Vec2::new(0.0, -0.012),
                max: Vec2::new(0.4, 0.012),
            },
        });
        let eye = Vec3::new(0.0, 1.6, 0.0);
        let centre = shape.centre();
        assert!(centre.distance(Vec3::new(0.0, 1.0, -0.4)) < 1e-6);
        let ray = Ray::new(eye, centre - eye).unwrap();
        assert!((shape.intersect(&ray).unwrap() - eye.distance(centre)).abs() < 1e-5);
        // Beside it in its plane, along it, and turned away from it, it is missed.
        let beside = Ray::new(eye, centre + Vec3::X * 0.25 - eye).unwrap();
        assert_eq!(shape.intersect(&beside), None);
        let along = Ray::new(centre + Vec3::Y, Vec3::X).unwrap();
        assert_eq!(shape.intersect(&along), None);
        let away = Ray::new(eye, eye - centre).unwrap();
        assert_eq!(shape.intersect(&away), None);
    }

    #[test]
    fn the_nearest_target_wins_and_ties_keep_the_first() {
        let targets = [
            sphere("far", 0.0, -3.0),
            sphere("near", 0.0, -1.0),
            sphere("aside", 1.0, -1.0),
        ];
        let hit = nearest(&forward(), &targets).unwrap();
        assert_eq!(hit.key, "near");
        assert!(hit.point.distance(Vec3::new(0.0, 1.0, -0.9)) < 1e-5);
        let twins = [sphere("first", 0.0, -1.0), sphere("second", 0.0, -1.0)];
        assert_eq!(nearest(&forward(), &twins).unwrap().key, "first");
        assert_eq!(nearest::<&str>(&forward(), &[]), None);
    }

    #[test]
    fn hovering_highlights_without_choosing() {
        let targets = [sphere("space", 0.0, -2.0)];
        let mut pointer = Pointer::default();
        let pointing = pointer.update(Some(forward()), Some(0.0), &targets);
        assert_eq!(pointing.hit.as_ref().unwrap().key, "space");
        assert_eq!(pointing.chosen, None);
        assert!(!pointing.attracting);
        assert_eq!(pointer.hovered(), Some(&"space"));
        assert!(
            pointing
                .endpoint(5.0)
                .unwrap()
                .distance(Vec3::new(0.0, 1.0, -1.9))
                < 1e-5
        );
        let away = Ray::new(Vec3::Y, Vec3::X).unwrap();
        let pointing = pointer.update(Some(away), Some(0.0), &targets);
        assert_eq!(pointing.hit, None);
        assert_eq!(pointer.hovered(), None);
        assert_eq!(pointing.endpoint(2.0), Some(Vec3::new(2.0, 1.0, 0.0)));
        let untracked = pointer.update(None, Some(0.0), &targets);
        assert_eq!(untracked.endpoint(2.0), None);
    }

    #[test]
    fn a_press_on_a_target_chooses_it_once_and_never_attracts() {
        let targets = [sphere("space", 0.0, -2.0)];
        let mut pointer = Pointer::default();
        let mut chosen = Vec::new();
        for trigger in [0.0, 0.3, 0.5, 0.6, 1.0, 0.8, 0.6, 1.0] {
            let pointing = pointer.update(Some(forward()), Some(trigger), &targets);
            assert!(!pointing.attracting, "a pull on a target must not attract");
            chosen.extend(pointing.chosen);
        }
        assert_eq!(chosen, ["space"], "held is not pressed again");
        // Released, then pressed again: a second choice.
        pointer.update(Some(forward()), Some(0.05), &targets);
        let pointing = pointer.update(Some(forward()), Some(0.9), &targets);
        assert_eq!(pointing.chosen, Some("space"));
    }

    #[test]
    fn a_pull_away_from_targets_attracts_even_when_it_crosses_one() {
        let targets = [sphere("space", 0.0, -2.0)];
        let away = Ray::new(Vec3::Y, Vec3::X).unwrap();
        let mut pointer = Pointer::default();
        assert!(!pointer.update(Some(away), Some(0.15), &targets).attracting);
        assert!(pointer.update(Some(away), Some(0.3), &targets).attracting);
        // Dragging the swarm across a target neither chooses it nor stops attracting.
        let across = pointer.update(Some(forward()), Some(1.0), &targets);
        assert!(across.attracting);
        assert_eq!(across.chosen, None);
        assert_eq!(across.hit.unwrap().key, "space");
        // A trigger resting between release and pull keeps attracting.
        assert!(pointer.update(Some(away), Some(0.15), &targets).attracting);
        assert!(!pointer.update(Some(away), Some(0.05), &targets).attracting);
    }

    #[test]
    fn with_nothing_to_point_at_the_trigger_attracts_as_before() {
        let mut pointer: Pointer<&str> = Pointer::default();
        let pointing = pointer.update(Some(forward()), Some(1.0), &[]);
        assert!(pointing.attracting);
        assert_eq!(pointing.chosen, None);
        let untracked = pointer.update(None, Some(1.0), &[]);
        assert!(untracked.attracting);
    }

    #[test]
    fn a_press_that_slides_off_its_target_chooses_nothing() {
        let targets = [sphere("space", 0.0, -2.0)];
        let away = Ray::new(Vec3::Y, Vec3::X).unwrap();
        let mut pointer = Pointer::default();
        pointer.update(Some(forward()), Some(0.3), &targets);
        let pointing = pointer.update(Some(away), Some(1.0), &targets);
        assert_eq!(pointing.chosen, None);
        assert!(!pointing.attracting);
    }

    #[test]
    fn inactive_or_invalid_triggers_and_release_end_the_pull() {
        let targets = [sphere("space", 0.0, -2.0)];
        let mut pointer = Pointer::default();
        pointer.update(Some(forward()), Some(0.3), &targets);
        // Losing the trigger, as on focus loss, ends the pull without choosing.
        assert_eq!(pointer.update(Some(forward()), None, &targets).chosen, None);
        let pointing = pointer.update(Some(forward()), Some(1.0), &targets);
        assert_eq!(pointing.chosen, Some("space"));
        pointer.release();
        assert_eq!(pointer.hovered(), None);
        assert!(
            !pointer
                .update(Some(forward()), Some(f32::NAN), &targets)
                .attracting
        );
        // After release, a held trigger is a new pull.
        assert_eq!(
            pointer.update(Some(forward()), Some(1.0), &targets).chosen,
            Some("space")
        );
    }
}
