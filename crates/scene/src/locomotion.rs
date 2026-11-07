// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Travel beyond the room: the locomotion profile an environment declares, its walkable surfaces and where a ray lands on them, the participant's [`Rig`], and [`Travel`], which turns the left stick and the ray into teleports, snap turns and rising. Pure, so that it is tested without a device; the frame step applies the rig to every pose. See `design/locomotion.md`.
use glam::{Mat4, Quat, Vec2, Vec3};

use crate::{environment::Environment, pointing::Ray};

/// How far a snap turn turns, in radians.
pub const SNAP_TURN: f32 = std::f32::consts::PI / 6.0;
/// How fast flying rises, in metres a second.
pub const ASCENT_SPEED: f32 = 0.6;
/// The furthest a landing can be from the ray's origin, horizontally, in metres.
pub const REACH: f32 = 15.0;
/// The nearest a landing can be along the ray, in metres.
pub const NEAREST: f32 = 0.25;
/// How far the stick moves from the middle before a push counts.
const STICK: f32 = 0.6;
/// How near the middle the stick returns before it turns again.
const RECENTRED: f32 = 0.3;
/// How far below the horizontal a ray must point to land anywhere, as the sine of the angle.
const DOWNWARDS: f32 = 0.02;

/// A horizontal surface a participant may land on, in the space's scene coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum Surface {
    /// A rectangle aligned with the scene's axes, centred at `centre`, `half` its size along X and Z.
    Rectangle { centre: Vec3, half: Vec2 },
    /// A disc centred at `centre`.
    Disc { centre: Vec3, radius: f32 },
}

impl Surface {
    /// A rectangle centred at `centre`, at its height, `size` across along X and Z.
    pub fn rectangle(centre: Vec3, size: Vec2) -> Self {
        Self::Rectangle {
            centre,
            half: size.abs() * 0.5,
        }
    }

    /// A disc centred at `centre`, at its height.
    pub fn disc(centre: Vec3, radius: f32) -> Self {
        Self::Disc {
            centre,
            radius: radius.abs(),
        }
    }

    /// The surface's height.
    pub fn height(&self) -> f32 {
        match *self {
            Self::Rectangle { centre, .. } | Self::Disc { centre, .. } => centre.y,
        }
    }

    /// Whether `point`, seen from above, is on the surface.
    pub fn contains(&self, point: Vec3) -> bool {
        match *self {
            Self::Rectangle { centre, half } => {
                (point.x - centre.x).abs() <= half.x && (point.z - centre.z).abs() <= half.y
            }
            Self::Disc { centre, radius } => {
                Vec2::new(point.x - centre.x, point.z - centre.z).length() <= radius
            }
        }
    }
}

/// How an environment lets its participants travel beyond the room.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub enum Locomotion {
    /// Room scale, as before: no artificial travel.
    #[default]
    None,
    /// Teleport onto the walkable `surfaces`, and turn in snaps.
    Teleport { surfaces: Vec<Surface> },
    /// Teleport and turn, and rise as well, never above `ceiling`, the highest the room's floor goes.
    Fly {
        surfaces: Vec<Surface>,
        ceiling: f32,
    },
}

impl Locomotion {
    /// Teleport onto `surfaces`, and turn in snaps.
    pub fn teleport(surfaces: Vec<Surface>) -> Self {
        Self::Teleport { surfaces }
    }

    /// Teleport onto `surfaces`, turn in snaps, and rise up to `ceiling`.
    pub fn fly(surfaces: Vec<Surface>, ceiling: f32) -> Self {
        Self::Fly { surfaces, ceiling }
    }

    /// Whether the participant travels at all.
    pub fn travels(&self) -> bool {
        !matches!(self, Self::None)
    }

    /// The highest the room's floor rises to, where it flies.
    pub fn ceiling(&self) -> Option<f32> {
        match self {
            Self::Fly { ceiling, .. } => Some(*ceiling),
            _ => None,
        }
    }

    fn surfaces(&self) -> &[Surface] {
        match self {
            Self::None => &[],
            Self::Teleport { surfaces } | Self::Fly { surfaces, .. } => surfaces,
        }
    }

    /// Where `ray` lands: on the first surface it meets from above, if that is at least [`NEAREST`] along it and at most [`REACH`] from its origin horizontally, or `None`. A surface nearer than that still hides those beneath it.
    pub fn landing(&self, ray: &Ray) -> Option<Vec3> {
        if ray.direction.y > -DOWNWARDS {
            return None;
        }
        let (distance, point) = self
            .surfaces()
            .iter()
            .filter_map(|surface| {
                let distance = (surface.height() - ray.origin.y) / ray.direction.y;
                let point = ray.at(distance);
                (distance >= 0.0 && surface.contains(point))
                    .then_some((distance, Vec3::new(point.x, surface.height(), point.z)))
            })
            .min_by(|(a, _), (b, _)| a.total_cmp(b))?;
        let across = Vec2::new(point.x - ray.origin.x, point.z - ray.origin.z).length();
        (distance >= NEAREST && across <= REACH).then_some(point)
    }
}

/// Where the participant's room is in the space's scene: a translation and a turn about the vertical. The room's floor is at its height.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rig {
    translation: Vec3,
    yaw: f32,
}

impl Rig {
    /// The transform from the room to the scene.
    pub fn matrix(&self) -> Mat4 {
        Mat4::from_rotation_translation(Quat::from_rotation_y(self.yaw), self.translation)
    }

    /// How high the room's floor is in the scene.
    pub fn floor(&self) -> f32 {
        self.translation.y
    }

    /// Move the room so that `head`, in the scene, stands above `landing` at the same height above the floor, with the floor at the landing's height.
    pub fn teleport(&mut self, landing: Vec3, head: Vec3) {
        self.translation += Vec3::new(landing.x - head.x, 0.0, landing.z - head.z);
        self.translation.y = landing.y;
    }

    /// Turn the room by `angle` about `head`, in the scene, which stays where it is.
    pub fn turn(&mut self, angle: f32, head: Vec3) {
        let room = self.matrix().inverse().transform_point3(head);
        self.yaw = (self.yaw + angle + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU)
            - std::f32::consts::PI;
        self.translation = head - Quat::from_rotation_y(self.yaw) * room;
    }

    /// Raise the room by `distance`, never above `ceiling`, nor lower than it is.
    pub fn ascend(&mut self, distance: f32, ceiling: f32) {
        self.translation.y = (self.translation.y + distance).min(ceiling.max(self.translation.y));
    }
}

/// Where travel applies: the space the participant is in, by its identifier, and its environment.
pub(crate) type Place<'a> = Option<(&'a str, Environment)>;

/// What travel needs each frame.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Input<'a> {
    /// The space the participant is in, or `None` outside a space.
    pub place: Place<'a>,
    pub locomotion: &'a Locomotion,
    /// The left stick.
    pub stick: Vec2,
    /// The pointing ray, in the scene.
    pub ray: Option<Ray>,
    /// The head, in the scene.
    pub head: Option<Vec3>,
    /// Nothing else takes the stick and the ray: in a space, focused, neither the hand menu nor the transcript showing, and no card carried.
    pub active: bool,
    /// Seconds since the last frame.
    pub dt: f32,
}

/// What travel did this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Outcome {
    /// A teleport is being aimed.
    pub aiming: bool,
    /// Where it would land.
    pub landing: Option<Vec3>,
    /// It landed this frame.
    pub teleported: bool,
    /// The rig turned this frame.
    pub turned: bool,
}

/// The participant's rig, and the teleport being aimed, between frames.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Travel {
    rig: Rig,
    /// The space and environment the rig belongs to.
    place: Option<(String, Environment)>,
    aiming: bool,
    landing: Option<Vec3>,
    turned: bool,
}

impl Travel {
    /// Where the participant's room is in the scene, as it was last settled.
    #[cfg(test)]
    pub fn rig(&self) -> Rig {
        self.rig
    }

    /// Drop a teleport being aimed, as on losing focus; the next turn needs the stick back near the middle.
    pub fn cancel(&mut self) {
        self.aiming = false;
        self.landing = None;
        self.turned = true;
    }

    /// Where the participant's room is in the scene at `place`, where the profile is `locomotion`: travel belongs to one environment of one space, so anywhere else, or without travel, the room is back home.
    pub fn settle(&mut self, place: Place, locomotion: &Locomotion) -> Rig {
        let here = self
            .place
            .as_ref()
            .map(|(space, environment)| (space.as_str(), *environment));
        if here != place || place.is_none() || !locomotion.travels() {
            *self = Self {
                place: place.map(|(space, environment)| (space.to_owned(), environment)),
                ..Self::default()
            };
        }
        self.rig
    }

    /// Move on a frame, as `input` finds things.
    pub fn update(&mut self, input: Input) -> Outcome {
        self.settle(input.place, input.locomotion);
        if !input.locomotion.travels() || !input.active {
            self.cancel();
            return Outcome::default();
        }
        let stick = if input.stick.is_finite() {
            input.stick
        } else {
            Vec2::ZERO
        };
        let mut outcome = Outcome::default();
        // Left or right turns once per push.
        if stick.x.abs() < RECENTRED {
            self.turned = false;
        }
        if stick.x.abs() > STICK && stick.x.abs() > stick.y.abs() && !self.turned {
            self.turned = true;
            if let Some(head) = input.head {
                self.rig.turn(-stick.x.signum() * SNAP_TURN, head);
                outcome.turned = true;
            }
        }
        // Forward aims while the ray points, and releasing it lands where it aimed, if the ray is still there: losing it, as a controller does that stops being tracked, cancels.
        let forward = stick.y > STICK && stick.y >= stick.x.abs();
        match input.ray.filter(|_| forward) {
            Some(ray) => {
                self.aiming = true;
                self.landing = input.locomotion.landing(&ray);
            }
            None => {
                if let (true, false, Some(landing), Some(head), Some(_)) =
                    (self.aiming, forward, self.landing, input.head, input.ray)
                {
                    self.rig.teleport(landing, head);
                    outcome.teleported = true;
                }
                self.aiming = false;
                self.landing = None;
            }
        }
        // Back rises, where the environment flies.
        if let Some(ceiling) = input.locomotion.ceiling()
            && stick.y < -STICK
            && -stick.y >= stick.x.abs()
            && input.dt.is_finite()
        {
            self.rig.ascend(ASCENT_SPEED * input.dt.max(0.0), ceiling);
        }
        outcome.aiming = self.aiming;
        outcome.landing = self.landing;
        outcome
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEAD: Vec3 = Vec3::new(0.3, 1.7, -0.2);
    const PLACE: Place = Some(("space-1", Environment::new("floor")));

    fn floors() -> Locomotion {
        Locomotion::teleport(vec![
            Surface::rectangle(Vec3::ZERO, Vec2::new(20.0, 20.0)),
            // A balcony, 3 m up, to the right.
            Surface::rectangle(Vec3::new(4.0, 3.0, 0.0), Vec2::new(2.0, 4.0)),
            Surface::disc(Vec3::new(-6.0, 1.0, 0.0), 1.0),
        ])
    }

    fn ray(origin: Vec3, towards: Vec3) -> Ray {
        Ray::new(origin, towards - origin).unwrap()
    }

    fn close(a: Vec3, b: Vec3) -> bool {
        a.distance(b) < 1e-4
    }

    #[test]
    fn surfaces_hold_what_is_above_them() {
        let rectangle = Surface::rectangle(Vec3::new(1.0, 2.0, 3.0), Vec2::new(-2.0, 4.0));
        assert_eq!(rectangle.height(), 2.0);
        assert!(rectangle.contains(Vec3::new(1.9, 0.0, 4.9)));
        assert!(!rectangle.contains(Vec3::new(2.1, 2.0, 3.0)));
        assert!(!rectangle.contains(Vec3::new(1.0, 2.0, 5.1)));
        let disc = Surface::disc(Vec3::new(0.0, 1.0, 0.0), -2.0);
        assert_eq!(disc.height(), 1.0);
        assert!(disc.contains(Vec3::new(1.4, 9.0, 1.4)));
        assert!(!disc.contains(Vec3::new(1.5, 1.0, 1.5)));
    }

    #[test]
    fn a_ray_lands_on_the_first_surface_it_meets_from_above() {
        let floors = floors();
        let eye = Vec3::new(0.0, 1.5, 0.0);
        // On the floor ahead.
        let landing = floors
            .landing(&ray(eye, Vec3::new(0.0, 0.0, -3.0)))
            .unwrap();
        assert!(close(landing, Vec3::new(0.0, 0.0, -3.0)));
        // On the disc, which stands above the floor beneath it.
        let landing = floors
            .landing(&ray(eye, Vec3::new(-6.0, 1.0, 0.5)))
            .unwrap();
        assert!(close(landing, Vec3::new(-6.0, 1.0, 0.5)));
        // The balcony, above the eye, cannot be reached from below; the floor beyond it can.
        let landing = floors.landing(&ray(eye, Vec3::new(5.0, 0.0, 0.0))).unwrap();
        assert_eq!(landing.y, 0.0);
        // From the balcony, it hides the floor beneath it, and the floor beyond it is reached.
        let up = Vec3::new(4.0, 4.6, 0.0);
        let landing = floors.landing(&ray(up, Vec3::new(4.5, 0.0, 0.5))).unwrap();
        assert!(close(
            landing.with_y(0.0),
            Vec3::new(4.0, 0.0, 0.0).lerp(Vec3::new(4.5, 0.0, 0.5), 1.6 / 4.6)
        ));
        assert_eq!(landing.y, 3.0);
        let landing = floors.landing(&ray(up, Vec3::new(8.0, 0.0, 0.0))).unwrap();
        assert!(close(landing, Vec3::new(8.0, 0.0, 0.0)));
        // Just above the balcony, it is too near to land on, and still hides the floor beneath it.
        let low = Vec3::new(4.0, 3.1, 0.0);
        assert_eq!(floors.landing(&ray(low, Vec3::new(4.1, 3.0, 0.0))), None);
    }

    #[test]
    fn some_rays_land_nowhere() {
        let floors = floors();
        let eye = Vec3::new(0.0, 1.5, 0.0);
        // Level, upwards, off every surface, too near and too far.
        assert_eq!(floors.landing(&ray(eye, Vec3::new(0.0, 1.5, -1.0))), None);
        assert_eq!(floors.landing(&ray(eye, Vec3::new(0.0, 3.0, -1.0))), None);
        assert_eq!(floors.landing(&ray(eye, Vec3::new(0.0, 0.0, -11.0))), None);
        assert_eq!(
            floors.landing(&ray(Vec3::new(0.0, 0.2, 0.0), Vec3::new(0.0, 0.0, -0.01))),
            None
        );
        let wide = Locomotion::teleport(vec![Surface::disc(Vec3::ZERO, 100.0)]);
        assert!(
            wide.landing(&ray(eye, Vec3::new(0.0, 0.0, -14.0)))
                .is_some()
        );
        assert_eq!(wide.landing(&ray(eye, Vec3::new(0.0, 0.0, -16.0))), None);
        // From high up, straight down is within reach.
        assert!(
            wide.landing(&ray(Vec3::new(0.0, 30.0, 0.0), Vec3::new(1.0, 0.0, 0.0)))
                .is_some()
        );
        assert_eq!(Locomotion::None.landing(&ray(eye, Vec3::ZERO)), None);
    }

    #[test]
    fn profiles_say_whether_they_travel_and_fly() {
        assert!(!Locomotion::default().travels());
        assert_eq!(Locomotion::default().ceiling(), None);
        assert!(floors().travels());
        assert_eq!(floors().ceiling(), None);
        let fly = Locomotion::fly(vec![Surface::disc(Vec3::ZERO, 5.0)], 10.0);
        assert!(fly.travels());
        assert_eq!(fly.ceiling(), Some(10.0));
        // Flying lands on its surfaces as teleporting does.
        let down = ray(Vec3::new(0.0, 8.0, 0.0), Vec3::new(1.0, 0.0, 0.0));
        assert!(close(fly.landing(&down).unwrap(), Vec3::new(1.0, 0.0, 0.0)));
    }

    #[test]
    fn teleports_keep_the_head_s_height_and_turns_keep_the_head_in_place() {
        let mut rig = Rig::default();
        let landing = Vec3::new(4.0, 3.0, 1.0);
        rig.teleport(landing, rig.matrix().transform_point3(HEAD));
        let head = rig.matrix().transform_point3(HEAD);
        assert!(close(head, landing + Vec3::Y * HEAD.y));
        assert_eq!(rig.floor(), 3.0);
        rig.turn(SNAP_TURN, head);
        assert!(close(rig.matrix().transform_point3(HEAD), head));
        assert_eq!(rig.floor(), 3.0);
        // The room's forward, -Z, turned 30° to the left.
        let forward = rig.matrix().transform_vector3(-Vec3::Z);
        assert!(close(
            forward,
            Vec3::new(-0.5, 0.0, -(3.0_f32.sqrt()) / 2.0)
        ));
        // Back down on the floor, at the same height above it.
        rig.teleport(
            Vec3::new(-2.0, 0.0, 0.0),
            rig.matrix().transform_point3(HEAD),
        );
        assert!((rig.matrix().transform_point3(HEAD).y - HEAD.y).abs() < 1e-5);
        // Turning all the way round stays within a turn.
        for _ in 0..13 {
            rig.turn(SNAP_TURN, Vec3::ZERO);
        }
        assert!(rig.yaw.abs() <= std::f32::consts::PI + 1e-5);
    }

    #[test]
    fn rising_stops_at_the_ceiling() {
        let mut rig = Rig::default();
        rig.ascend(ASCENT_SPEED * 2.0, 10.0);
        assert!((rig.floor() - 1.2).abs() < 1e-5);
        rig.ascend(100.0, 10.0);
        assert_eq!(rig.floor(), 10.0);
        // A ceiling below where the floor already is keeps it there.
        rig.teleport(Vec3::new(0.0, 12.0, 0.0), Vec3::ZERO);
        rig.ascend(1.0, 10.0);
        assert_eq!(rig.floor(), 12.0);
    }

    fn input(locomotion: &Locomotion, stick: Vec2, ray: Option<Ray>) -> Input<'_> {
        Input {
            place: PLACE,
            locomotion,
            stick,
            ray,
            head: Some(HEAD),
            active: true,
            dt: 0.5,
        }
    }

    #[test]
    fn forward_aims_and_releasing_it_lands_once() {
        let floors = floors();
        let mut travel = Travel::default();
        let aim = Some(ray(Vec3::new(0.3, 1.2, -0.4), Vec3::new(0.0, 0.0, -4.0)));
        let outcome = travel.update(input(&floors, Vec2::new(0.1, 0.9), aim));
        assert!(outcome.aiming && !outcome.teleported);
        assert!(close(outcome.landing.unwrap(), Vec3::new(0.0, 0.0, -4.0)));
        assert_eq!(travel.rig(), Rig::default(), "aiming does not move");
        let outcome = travel.update(input(&floors, Vec2::ZERO, aim));
        assert!(outcome.teleported && !outcome.aiming);
        assert_eq!(outcome.landing, None);
        let head = travel.rig().matrix().transform_point3(HEAD);
        assert!(close(head, Vec3::new(0.0, HEAD.y, -4.0)));
        // Released, it does nothing more.
        let outcome = travel.update(input(&floors, Vec2::ZERO, aim));
        assert_eq!(outcome, Outcome::default());
    }

    #[test]
    fn releasing_with_nowhere_to_land_or_after_losing_the_ray_does_nothing() {
        let floors = floors();
        let mut travel = Travel::default();
        let up = Some(ray(HEAD, HEAD + Vec3::Y));
        let outcome = travel.update(input(&floors, Vec2::Y, up));
        assert!(outcome.aiming);
        assert_eq!(outcome.landing, None);
        assert!(!travel.update(input(&floors, Vec2::ZERO, up)).teleported);
        // Aimed, then released as the ray is lost.
        let aim = Some(ray(HEAD, Vec3::new(0.0, 0.0, -3.0)));
        assert!(
            travel
                .update(input(&floors, Vec2::Y, aim))
                .landing
                .is_some()
        );
        assert!(!travel.update(input(&floors, Vec2::ZERO, None)).teleported);
        // Aimed, then the ray is lost while the stick is held.
        let aim = Some(ray(HEAD, Vec3::new(0.0, 0.0, -3.0)));
        assert!(
            travel
                .update(input(&floors, Vec2::Y, aim))
                .landing
                .is_some()
        );
        assert!(!travel.update(input(&floors, Vec2::Y, None)).aiming);
        assert!(!travel.update(input(&floors, Vec2::ZERO, aim)).teleported);
        // Aimed, then the hand menu shows.
        travel.update(input(&floors, Vec2::Y, aim));
        let inactive = Input {
            active: false,
            ..input(&floors, Vec2::Y, aim)
        };
        assert_eq!(travel.update(inactive), Outcome::default());
        assert!(!travel.update(input(&floors, Vec2::ZERO, aim)).teleported);
        // Aimed, then focus is lost.
        travel.update(input(&floors, Vec2::Y, aim));
        travel.cancel();
        assert!(!travel.update(input(&floors, Vec2::ZERO, aim)).teleported);
        // Without a head, a release lands nowhere.
        travel.update(input(&floors, Vec2::Y, aim));
        let headless = Input {
            head: None,
            ..input(&floors, Vec2::ZERO, aim)
        };
        assert!(!travel.update(headless).teleported);
        assert_eq!(travel.rig(), Rig::default());
    }

    #[test]
    fn left_and_right_turn_once_per_push() {
        let floors = floors();
        let mut travel = Travel::default();
        let outcome = travel.update(input(&floors, Vec2::new(0.9, 0.2), None));
        assert!(outcome.turned);
        let right = travel.rig();
        // Turning right turns the room's forward to the right.
        assert!(right.matrix().transform_vector3(-Vec3::Z).x > 0.4);
        assert!(close(right.matrix().transform_point3(HEAD), HEAD));
        // Held, or eased off but not back near the middle, it does not turn again.
        assert!(
            !travel
                .update(input(&floors, Vec2::new(0.9, 0.0), None))
                .turned
        );
        assert!(
            !travel
                .update(input(&floors, Vec2::new(0.4, 0.0), None))
                .turned
        );
        assert!(
            !travel
                .update(input(&floors, Vec2::new(0.9, 0.0), None))
                .turned
        );
        travel.update(input(&floors, Vec2::ZERO, None));
        assert!(
            travel
                .update(input(&floors, Vec2::new(-0.9, 0.0), None))
                .turned
        );
        assert!((travel.rig().yaw).abs() < 1e-5, "back where it was");
        // A push that is more forward than sideways aims instead.
        travel.update(input(&floors, Vec2::ZERO, None));
        assert!(
            !travel
                .update(input(&floors, Vec2::new(0.7, 0.8), None))
                .turned
        );
        // Without a head, a push turns nothing, and still counts as the push.
        travel.update(input(&floors, Vec2::ZERO, None));
        let headless = Input {
            head: None,
            ..input(&floors, Vec2::X, None)
        };
        assert!(!travel.update(headless).turned);
        assert!(!travel.update(input(&floors, Vec2::X, None)).turned);
        // After focus returns, the stick must come back to the middle first.
        travel.update(input(&floors, Vec2::ZERO, None));
        travel.cancel();
        assert!(!travel.update(input(&floors, Vec2::X, None)).turned);
        // A stick that reads nonsense is in the middle.
        travel.update(input(&floors, Vec2::NAN, None));
        assert!(travel.update(input(&floors, Vec2::X, None)).turned);
    }

    #[test]
    fn back_rises_only_where_the_environment_flies() {
        let floors = floors();
        let mut travel = Travel::default();
        travel.update(input(&floors, Vec2::NEG_Y, None));
        assert_eq!(travel.rig().floor(), 0.0);
        let fly = Locomotion::fly(Vec::new(), 1.0);
        travel.update(input(&fly, Vec2::NEG_Y, None));
        assert!((travel.rig().floor() - 0.3).abs() < 1e-5);
        // A frame without a sensible length, or a push more sideways, rises nothing.
        let nonsense = Input {
            dt: f32::NAN,
            ..input(&fly, Vec2::NEG_Y, None)
        };
        travel.update(nonsense);
        travel.update(input(&fly, Vec2::new(0.9, -0.7), None));
        assert!((travel.rig().floor() - 0.3).abs() < 1e-5);
        for _ in 0..10 {
            travel.update(input(&fly, Vec2::NEG_Y, None));
        }
        assert_eq!(travel.rig().floor(), 1.0);
    }

    #[test]
    fn the_rig_goes_home_on_leaving_changing_environment_or_without_travel() {
        let floors = floors();
        let mut travel = Travel::default();
        let turn = |travel: &mut Travel, input: Input| {
            travel.update(Input {
                stick: Vec2::ZERO,
                ..input
            });
            travel.update(input);
        };
        turn(&mut travel, input(&floors, Vec2::X, None));
        assert_ne!(travel.rig(), Rig::default());
        // Inactive, as while the hand menu shows, it stays where it is.
        travel.update(Input {
            active: false,
            ..input(&floors, Vec2::ZERO, None)
        });
        assert_ne!(travel.rig(), Rig::default());
        // Leaving the space.
        travel.update(Input {
            place: None,
            ..input(&floors, Vec2::ZERO, None)
        });
        assert_eq!(travel.rig(), Rig::default());
        // Another space in the same environment.
        turn(&mut travel, input(&floors, Vec2::X, None));
        assert_ne!(travel.rig(), Rig::default());
        assert_eq!(
            travel.settle(Some(("space-2", Environment::new("floor"))), &floors),
            Rig::default()
        );
        // Another environment.
        turn(&mut travel, input(&floors, Vec2::X, None));
        travel.update(Input {
            place: Some(("space-1", Environment::SKY)),
            ..input(&floors, Vec2::ZERO, None)
        });
        assert_eq!(travel.rig(), Rig::default());
        // No travel in the same environment, and nothing moves then.
        turn(&mut travel, input(&floors, Vec2::X, None));
        let none = Locomotion::None;
        travel.update(Input {
            place: Some(("space-1", Environment::SKY)),
            ..input(&none, Vec2::X, None)
        });
        assert_eq!(travel.rig(), Rig::default());
        assert_eq!(
            travel.update(Input {
                place: Some(("space-1", Environment::SKY)),
                ..input(&none, Vec2::X, None)
            }),
            Outcome::default()
        );
    }
}
