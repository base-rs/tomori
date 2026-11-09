// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Where participants arrive, and where Tomori's agent and its delegated workers stand in a space: the anchors the
//! space's environment gives, `arrival`, `agent` and `workers`, which the server sends in the space's state, where a
//! participant summoned the agent, and the placement of workers about the workers anchor, which an environment's
//! client crate may register beside its scenery. Every anchor faces along its +Z, the way it looks. Without them, the
//! layout is exactly the default: home at the origin facing −Z, the swarm at [`crate::swarm::CENTER`], facing home, and
//! the workers on [`slot_position`]'s arc about home. See `design/environments.md`.
use std::sync::Arc;

use glam::{Quat, Vec3};
use tomori_protocol::Pose;

use crate::delegation::{State, slot_position};

/// How far from the space's middle, on each axis, an anchor may stand, in metres, as the server bounds it.
pub const REACH: f32 = 20.0;

/// How far from the space's middle, on each axis, a placement may stand a worker, in metres; beyond it, or where it
/// is not finite, the worker stands where the default arc would put it.
pub const PLACEMENT_REACH: f32 = 90.0;

/// A place and an upright facing, turned only about the vertical, in the space's coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Anchor {
    pub position: Vec3,
    pub orientation: Quat,
}

impl Anchor {
    /// Where the swarm's middle stands by default, facing home.
    pub const AGENT: Self = Self {
        position: crate::swarm::CENTER,
        orientation: Quat::IDENTITY,
    };

    /// Nowhere and unturned: the carriage of an environment whose participants arrive at its own origin, facing −Z,
    /// which leaves everything where it is.
    pub const HOME: Self = Self {
        position: Vec3::ZERO,
        orientation: Quat::IDENTITY,
    };

    /// The carriage from an environment's own coordinates into a space's, for an environment whose participants
    /// arrive at `arrival`, in its own coordinates, facing along its +Z, as the server sends it, if it is finite, within
    /// 1 km and has an orientation: the anchor whose [`Self::place`] carries a point there into the space, where home is
    /// the origin, facing −Z. See [`tomori_protocol::Pose::carriage`].
    pub fn carriage(arrival: &Pose) -> Option<Self> {
        let arrival = Self::within(arrival, 1000.0)?;
        let carriage = Pose::carriage(&Pose {
            position: arrival.position.to_array(),
            orientation: arrival.orientation.to_array(),
        });
        Some(Self {
            position: Vec3::from_array(carriage.position),
            orientation: Quat::from_array(carriage.orientation),
        })
    }

    /// The way it faces, its +Z, as an angle about the vertical: 0 along +Z, a quarter turn along +X.
    pub fn yaw(&self) -> f32 {
        let ahead = self.orientation * Vec3::Z;
        ahead.x.atan2(ahead.z)
    }

    /// The point `point`, in the space, in the anchor's own frame: where [`Self::place`] would take it from.
    pub fn unplace(&self, point: Vec3) -> Vec3 {
        if *self == Self::HOME {
            point
        } else {
            self.orientation.inverse() * (point - self.position)
        }
    }

    /// The ray `ray`, in the space, in the anchor's own frame, as an environment's client side picks in its own
    /// coordinates.
    pub fn unplace_ray(&self, ray: &crate::pointing::Ray) -> crate::pointing::Ray {
        if *self == Self::HOME {
            *ray
        } else {
            crate::pointing::Ray {
                origin: self.unplace(ray.origin),
                direction: self.orientation.inverse() * ray.direction,
            }
        }
    }

    /// The anchor `pose` gives, as [`Self::from_pose`] makes it, but within `reach`.
    fn within(pose: &Pose, reach: f32) -> Option<Self> {
        let position = Vec3::from_array(pose.position);
        if !(position.is_finite() && position.abs().max_element() <= reach) {
            return None;
        }
        Self::from_pose(&Pose {
            position: [0.0; 3],
            ..*pose
        })
        .map(|anchor| Self { position, ..anchor })
    }

    /// The anchor `pose` gives, upright, if it is finite, within [`REACH`] and has an orientation, as the server
    /// bounds it. One tilted is turned to face the way its +Z faces, levelled, or, where that is straight up or down,
    /// the way its +X faces turned a quarter towards it.
    pub fn from_pose(pose: &Pose) -> Option<Self> {
        let position = Vec3::from_array(pose.position);
        let orientation = Quat::from_array(pose.orientation);
        let length = orientation.length();
        if !(position.is_finite()
            && position.abs().max_element() <= REACH
            && length.is_finite()
            && length > 0.5)
        {
            return None;
        }
        let orientation = orientation / length;
        if orientation.x.abs() < 1e-6 && orientation.z.abs() < 1e-6 {
            return Some(Self {
                position,
                orientation,
            });
        }
        let ahead = orientation * Vec3::Z;
        let (x, z) = if ahead.x.hypot(ahead.z) < 1e-3 {
            let right = orientation * Vec3::X;
            (-right.z, right.x)
        } else {
            (ahead.x, ahead.z)
        };
        Some(Self {
            position,
            orientation: Quat::from_rotation_y(x.atan2(z)),
        })
    }

    /// The point `local`, in the anchor's own frame, in the space: moved to it and turned with it. At
    /// [`Self::HOME`], it is `local` itself, and unturned, `local` moved, so that the default layout is exactly as it
    /// was.
    pub fn place(&self, local: Vec3) -> Vec3 {
        if *self == Self::HOME {
            local
        } else if self.orientation == Quat::IDENTITY {
            self.position + local
        } else {
            self.position + self.orientation * local
        }
    }
}

/// A worker that stands somewhere already, as a placement is told of it: its identity, its slot, its state and where
/// it stands.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct Placed<'a> {
    pub id: &'a str,
    pub slot: usize,
    pub state: State,
    pub position: Vec3,
}

impl<'a> Placed<'a> {
    pub fn new(id: &'a str, slot: usize, state: State, position: Vec3) -> Self {
        Self {
            id,
            slot,
            state,
            position,
        }
    }
}

/// A worker to be placed: its identity, the worker that delegated to it, or `None` for one the space's own agent
/// started, its state, its slot, which is remembered for its identity, from 0, the most prominent, and the workers
/// that stand already, where they stand, in the order of their slots, whether they arrived before it or are being
/// placed again with it.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct Placing<'a> {
    pub id: &'a str,
    pub parent: Option<&'a str>,
    pub state: State,
    pub slot: usize,
    pub workers: &'a [Placed<'a>],
}

impl<'a> Placing<'a> {
    /// The worker `id`, in `slot`, started by the space's own agent, its state unknown, with no other worker standing.
    pub fn new(id: &'a str, slot: usize) -> Self {
        Self {
            id,
            parent: None,
            state: State::Unknown,
            slot,
            workers: &[],
        }
    }
}

/// Where an environment's workers stand, about the workers anchor, `origin`, the middle of their arrangement, facing
/// along its +Z towards whoever watches it, as the agent does, by default the agent's home: registered by the
/// environment's client
/// crate beside its scenery, with `tomori-render`'s `Sceneries::set_worker_placement`. It is asked only as a worker
/// arrives, and again for every live worker, in the order of their slots, when the arrangement changes, so a live
/// worker never moves when the workers' titles, states, order or membership change, whatever it answers. It is the
/// build's own code, trusted as its scenery is, and runs on the frame step's thread, at most once for each of at most
/// 64 live workers in a frame, so it must be quick; one that panics, or answers a place that is not finite or beyond
/// [`PLACEMENT_REACH`], has its worker stand where [`HomeArc`] puts it. It may stand workers together: their signs then
/// stand beside or above one another. See `design/environments.md`.
pub trait WorkerPlacement: Send + Sync {
    /// Where `worker` stands, in the space's coordinates.
    fn place(&self, origin: Anchor, worker: &Placing<'_>) -> Vec3;
}

/// A worker placement, shared between the frame step and the spectator.
pub type Placement = Arc<dyn WorkerPlacement>;

/// The default placement: [`slot_position`]'s arc about home, which passes through the agent's default home, carried
/// with it to the workers anchor, the arrangement's middle, and turned with it, so that by default workers stand about
/// the agent's home as they stand about its default home, and at [`Anchor::AGENT`] exactly where they always have.
#[derive(Clone, Copy, Debug, Default)]
pub struct HomeArc;

impl HomeArc {
    /// Where the default arc stands `slot` about `origin`.
    pub fn at(origin: Anchor, slot: usize) -> Vec3 {
        if origin == Anchor::AGENT {
            slot_position(slot)
        } else {
            origin.place(slot_position(slot) - crate::swarm::CENTER)
        }
    }
}

impl WorkerPlacement for HomeArc {
    fn place(&self, origin: Anchor, worker: &Placing<'_>) -> Vec3 {
        Self::at(origin, worker.slot)
    }
}

/// Where `worker` stands about `origin`: as `placement` says, if it is given one, that does not panic, and its answer
/// is finite and within [`PLACEMENT_REACH`], or else where [`HomeArc`] puts it; and whether the placement panicked, so
/// that it is not called again.
pub fn place(origin: Anchor, placement: Option<&Placement>, worker: &Placing<'_>) -> (Vec3, bool) {
    let answer = placement.map(|placement| {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            placement.place(origin, worker)
        }))
    });
    let panicked = matches!(answer, Some(Err(_)));
    let position = answer
        .and_then(Result::ok)
        .filter(|position| position.is_finite() && position.abs().max_element() <= PLACEMENT_REACH)
        .unwrap_or_else(|| HomeArc.place(origin, worker));
    (position, panicked)
}

/// Where the agent and the workers stand in the joined space: the swarm at `agent`, where a participant summoned it or
/// else at its `home`; the workers about `workers`, by default its home, by `placement`, or by [`HomeArc`] without one;
/// and the `carriage` that takes the environment's own coordinates, its scenery's, walkable surfaces' and client
/// side's, into the space's.
#[derive(Clone)]
pub struct Arrangement {
    pub agent: Anchor,
    pub home: Anchor,
    pub workers: Anchor,
    pub placement: Option<Placement>,
    pub carriage: Anchor,
}

impl Arrangement {
    /// Whether a participant summoned the agent away from its home.
    pub fn summoned(&self) -> bool {
        self.agent != self.home
    }
}

impl Default for Arrangement {
    /// The default layout, as outside a space.
    fn default() -> Self {
        Self {
            agent: Anchor::AGENT,
            home: Anchor::AGENT,
            workers: Anchor::AGENT,
            placement: None,
            carriage: Anchor::HOME,
        }
    }
}

impl std::fmt::Debug for Arrangement {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Arrangement")
            .field("agent", &self.agent)
            .field("home", &self.home)
            .field("workers", &self.workers)
            .field("placement", &self.placement.is_some())
            .field("carriage", &self.carriage)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pose(position: [f32; 3], orientation: [f32; 4]) -> Pose {
        Pose {
            position,
            orientation,
        }
    }

    fn close(a: Vec3, b: Vec3) -> bool {
        a.distance(b) < 1e-5
    }

    #[test]
    fn an_anchor_is_bounded_and_made_upright() {
        let upright = [0.0, 0.0, 0.0, 1.0];
        assert_eq!(
            Anchor::from_pose(&pose([1.0, 2.0, -3.0], upright)),
            Some(Anchor {
                position: Vec3::new(1.0, 2.0, -3.0),
                orientation: Quat::IDENTITY,
            })
        );
        // Unnormalised, it is normalised.
        let doubled = Anchor::from_pose(&pose([0.0; 3], [0.0, 0.0, 0.0, 2.0])).unwrap();
        assert_eq!(doubled.orientation, Quat::IDENTITY);
        // Beyond reach, not finite, or without an orientation, it is none.
        for refused in [
            pose([0.0, 0.0, -20.5], upright),
            pose([f32::NAN, 0.0, 0.0], upright),
            pose([0.0; 3], [0.0; 4]),
            pose([0.0; 3], [f32::INFINITY, 0.0, 0.0, 1.0]),
        ] {
            assert_eq!(Anchor::from_pose(&refused), None);
        }
        // Tilted forward, it faces where its +Z does, levelled.
        let tilted = Quat::from_rotation_y(0.5) * Quat::from_rotation_x(0.3);
        let anchor = Anchor::from_pose(&pose([0.0; 3], tilted.to_array())).unwrap();
        assert!(anchor.orientation.angle_between(Quat::from_rotation_y(0.5)) < 1e-4);
        // Facing straight up, it faces where its +X does, turned a quarter.
        let up = Quat::from_rotation_y(0.5) * Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2);
        let anchor = Anchor::from_pose(&pose([0.0; 3], up.to_array())).unwrap();
        assert!(anchor.orientation.angle_between(Quat::from_rotation_y(0.5)) < 1e-3);
    }

    #[test]
    fn an_anchor_carries_points_with_it_and_home_leaves_them_as_they_are() {
        let local = Vec3::new(1.0, 0.5, -2.0);
        assert_eq!(Anchor::HOME.place(local), local);
        assert_eq!(Anchor::AGENT.place(local), crate::swarm::CENTER + local);
        let turned = Anchor {
            position: Vec3::new(5.0, 0.0, 0.0),
            orientation: Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
        };
        // Turned a quarter, ahead (−Z) is −X.
        assert!(close(
            turned.place(Vec3::new(0.0, 1.0, -2.0)),
            Vec3::new(3.0, 1.0, 0.0)
        ));
    }

    struct Fixed(Vec3);

    impl WorkerPlacement for Fixed {
        fn place(&self, origin: Anchor, worker: &Placing<'_>) -> Vec3 {
            origin.place(self.0) + Vec3::X * worker.workers.len() as f32
        }
    }

    #[test]
    fn the_default_is_the_arc_at_the_origin_and_a_placement_is_bounded() {
        let worker = Placing::new("ses_1", 3);
        assert_eq!(worker.parent, None);
        assert_eq!(worker.state, State::Unknown);
        assert_eq!(
            place(Anchor::AGENT, None, &worker),
            (slot_position(3), false)
        );
        let origin = Anchor {
            position: Vec3::new(0.0, 0.0, -6.0),
            orientation: Quat::IDENTITY,
        };
        // Carried with the arrangement's middle, 6 m further on.
        let further = Anchor {
            position: crate::swarm::CENTER + Vec3::new(0.0, 0.0, -6.0),
            orientation: Quat::IDENTITY,
        };
        let (moved, panicked) = place(further, None, &worker);
        assert!(close(moved, slot_position(3) + Vec3::new(0.0, 0.0, -6.0)) && !panicked);
        let fixed: Placement = Arc::new(Fixed(Vec3::new(1.0, 1.0, 1.0)));
        let others = [Placed::new("ses_0", 0, State::Running, Vec3::ZERO)];
        let mut placing = Placing::new("ses_1", 3);
        placing.workers = &others;
        assert_eq!(
            place(origin, Some(&fixed), &placing),
            (Vec3::new(2.0, 1.0, -5.0), false)
        );
        // Out of bounds or not finite, the arc's place.
        for wild in [Vec3::new(0.0, 0.0, 95.0), Vec3::NAN] {
            let wild: Placement = Arc::new(Fixed(wild));
            assert_eq!(
                place(Anchor::AGENT, Some(&wild), &worker),
                (slot_position(3), false)
            );
        }
        // One that panics, the arc's place, saying it panicked.
        struct Panics;
        impl WorkerPlacement for Panics {
            fn place(&self, _origin: Anchor, worker: &Placing<'_>) -> Vec3 {
                panic!("no place for {}", worker.id)
            }
        }
        let panics: Placement = Arc::new(Panics);
        assert_eq!(
            place(Anchor::AGENT, Some(&panics), &worker),
            (slot_position(3), true)
        );
        let arrangement = Arrangement::default();
        assert_eq!(
            (arrangement.agent, arrangement.home, arrangement.workers),
            (Anchor::AGENT, Anchor::AGENT, Anchor::AGENT)
        );
        assert!(format!("{arrangement:?}").contains("placement: false"));
        assert!(!arrangement.summoned());
        assert_eq!(arrangement.carriage, Anchor::HOME);
    }

    #[test]
    fn every_anchor_faces_along_its_z_and_facing_towards_turns_it_the_right_way() {
        // An agent made to face home with the protocol's helper faces home, as the default does, unturned.
        let agent = Pose::at(crate::swarm::CENTER.to_array()).facing_towards([0.0, 1.6, 0.0]);
        assert_eq!(Anchor::from_pose(&agent), Some(Anchor::AGENT));
        // The workers' arrangement made the same way stands exactly where the default does: the arc about home.
        let workers = Anchor::from_pose(&agent).unwrap();
        assert_eq!(HomeArc::at(workers, 2), slot_position(2));
        // To one side, facing home, its arc still curves about home: its middle slot stands nearer home than the
        // arrangement's middle, on the way to home, as the default's does.
        let side = Pose::at([2.3, 1.4, 0.0]).facing_towards([0.0, 1.4, 0.0]);
        let side = Anchor::from_pose(&side).unwrap();
        let yaw = side.yaw();
        assert!((yaw + std::f32::consts::FRAC_PI_2).abs() < 1e-5, "{yaw}");
        let ahead = side.place(Vec3::Z);
        assert!(close(ahead, Vec3::new(1.3, 1.4, 0.0)), "{ahead}");
        for slot in 0..8 {
            let at = HomeArc::at(side, slot);
            // On a circle of 3.2 m about home, as the default arc is about home.
            assert!(
                (Vec3::new(at.x, 0.0, at.z).length() - 3.2).abs() < 1e-3,
                "{slot}: {at}"
            );
        }
    }

    #[test]
    fn an_arrival_carries_the_environment_so_that_home_is_the_origin_facing_minus_z() {
        // By default, it leaves everything where it is.
        assert_eq!(Anchor::carriage(&Pose::HOME), Some(Anchor::HOME));
        // Arriving 8 m along +X, facing −X: the environment's origin is 8 m ahead of home.
        let street = Pose::at([8.0, 0.0, 0.0]).facing_towards([0.0; 3]);
        let carriage = Anchor::carriage(&street).unwrap();
        assert!(close(carriage.place(Vec3::ZERO), Vec3::new(0.0, 0.0, -8.0)));
        assert!(close(
            carriage.place(Vec3::new(8.0, 1.6, 0.0)),
            Vec3::new(0.0, 1.6, 0.0)
        ));
        // Not finite, or beyond a kilometre, none.
        assert_eq!(Anchor::carriage(&Pose::at([f32::NAN, 0.0, 0.0])), None);
        assert_eq!(Anchor::carriage(&Pose::at([1500.0, 0.0, 0.0])), None);
    }
}
