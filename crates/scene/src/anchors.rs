// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Where participants arrive, and where Tomori's agent and its delegated workers stand in a space: the anchors the
//! space's environment gives, `arrival`, `agent` and `workers`, which the server sends in the space's state, where a
//! participant summoned the agent, and the placement of workers about the workers anchor, which an environment's
//! client crate may register beside its scenery. Every anchor faces along its +Z, the way it looks. Without them, the
//! layout is exactly the default: home at the origin facing −Z, the swarm at [`crate::swarm::CENTER`], facing home, and
//! the workers on [`crate::delegation::slot_position`]'s arc about home. See `design/environments.md`.
use std::sync::Arc;

use glam::{Quat, Vec3};
use tomori_protocol::Pose;

use crate::delegation::State;

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
    /// Where the eyes are of whoever the agent faces, whom its entourage gathers about: at home, the eyes at home,
    /// [`EYES`]; summoned, those of whoever summoned it. The default arc curves about them.
    pub audience: Vec3,
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
            audience: EYES,
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

/// Where the eyes are, at home, of participants arriving: 1.6 m above home, as `tomori_environment::sightlines` takes
/// them to be.
pub const EYES: Vec3 = Vec3::new(0.0, 1.6, 0.0);

/// How far from whoever watches the arc stands at which its columns stand at their own angles, 40° from the agent and
/// 18⅔° apart, and its rows 85 cm apart, in metres; nearer, its columns stand as far apart along the arc as they do here,
/// so that their signs keep clear of one another and of the swarm, and further, its columns stand at those angles, its
/// rows spread in proportion, as its signs are drawn larger there.
pub const ARC_RADIUS: f32 = 3.2;

/// How near whoever watches the arc stands at least, horizontally, in metres: as near as the agent's default home, and
/// a summoned agent, stand (`tomori_protocol::entourage::SUMMONED`), so that its first columns stand in front of them.
pub const NEAREST_ARC: f32 = tomori_protocol::entourage::SUMMONED;

/// How far below the agent's middle the arc's first row stands, and how far apart its rows stand, in metres, at
/// [`ARC_RADIUS`] or nearer: the first row a little below the agent's middle, the second above it, so that the agent
/// stands within the band they make, then each row above the last, every one above the Tomori wall beneath the agent.
pub const ROW_BELOW: f32 = 0.25;
pub const ROW_APART: f32 = 0.85;

/// The lowest any worker stands above the floor, in metres.
const LOWEST: f32 = 0.3;

/// The default placement: one curved band about whoever watches, the `audience`, on which the arrangement's middle, the
/// agent, itself stands, in the middle of its gap: its radius is the horizontal distance between them, but at least
/// [`NEAREST_ARC`], and its workers stand along it to either side of the agent, alternately left and right, two rows to
/// each pair of columns, from 40° of the agent's bearing from their eyes, 18⅔° apart, or as far apart along the arc
/// where it stands nearer than [`ARC_RADIUS`], at about the agent's height: the first row [`ROW_BELOW`] below its
/// middle, the rest [`ROW_APART`] above one another, spread in proportion where it stands further than [`ARC_RADIUS`],
/// as its signs are drawn larger there, so that the Tomori wall stands beneath the band. Nearer than [`ARC_RADIUS`], the
/// band holds the first two rows of the columns that fit before where the last stands at [`ARC_RADIUS`], five at the
/// default home's 2.3 m, eighteen workers; the rest stand on the arc [`ARC_RADIUS`] from whoever watches, at their
/// columns' own angles, above and behind the band, the columns that do not fit from two rows up, so that as many
/// workers as there are look around and up. So the
/// workers stand about the agent, all about as far from whoever watches as it is, facing them, nothing behind it, as
/// they stood before environments placed the agent, at home and summoned alike; and at [`Anchor::AGENT`], watched from
/// [`EYES`], where [`crate::delegation::slot_position`] puts them.
#[derive(Clone, Copy, Debug, Default)]
pub struct HomeArc;

impl HomeArc {
    /// Where the default arc stands `slot` about whoever's eyes are at `audience`, through `origin`.
    pub fn at(origin: Anchor, audience: Vec3, slot: usize) -> Vec3 {
        let (side, column, row) = crate::delegation::slot_lane(slot);
        let across = Vec3::new(
            origin.position.x - audience.x,
            0.0,
            origin.position.z - audience.z,
        );
        // Towards the middle, or else, as from its very place, the way it faces them from.
        let ahead = across.try_normalize().unwrap_or_else(|| {
            let facing = origin.orientation * Vec3::Z;
            Vec3::new(-facing.x, 0.0, -facing.z)
                .try_normalize()
                .unwrap_or(Vec3::NEG_Z)
        });
        let right = Vec3::new(-ahead.z, 0.0, ahead.x);
        let radius = across.length().max(NEAREST_ARC);
        // Nearer than the arc's own radius, its columns stand as far apart along it as there, as many as fit before the
        // last column there; their rows above the first two, and the rest of the columns, two rows up, stand on the
        // arc's own radius, at their own angles, above and behind the band.
        let widen = (ARC_RADIUS / radius).max(1.0);
        let last = crate::delegation::slot_angle(COLUMNS - 1);
        let fit = (0..COLUMNS)
            .take_while(|column| crate::delegation::slot_angle(*column) * widen <= last + 1e-4)
            .count();
        let (radius, angle, level) = if fit < COLUMNS && (column >= fit || row >= 2) {
            let lift = if column >= fit { 2 } else { 0 };
            (
                radius.max(ARC_RADIUS),
                crate::delegation::slot_angle(column),
                row + lift,
            )
        } else {
            (radius, crate::delegation::slot_angle(column) * widen, row)
        };
        let direction = ahead * angle.cos() + right * (side * angle.sin());
        // Further than the arc's own radius, its rows spread in proportion, as its signs are drawn larger, so that no
        // sign reaches the worker above it.
        let spread = (radius / ARC_RADIUS).max(1.0);
        let y = (origin.position.y + (level as f32 * ROW_APART - ROW_BELOW) * spread)
            .max(tomori_protocol::entourage::FLOOR + LOWEST);
        Vec3::new(audience.x, y, audience.z) + direction * radius
    }
}

/// How many columns the arc has on each side of the agent.
const COLUMNS: usize = 8;

impl WorkerPlacement for HomeArc {
    fn place(&self, origin: Anchor, worker: &Placing<'_>) -> Vec3 {
        Self::at(origin, worker.audience, worker.slot)
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
/// else at its `home`; the workers about `workers`, the agent itself while it is summoned, or else the workers anchor,
/// by default its home, by `placement`, or by [`HomeArc`] without one, gathered about `audience`, the eyes of whoever
/// summoned it, or else those at home; and the `carriage` that takes the environment's own coordinates, its
/// scenery's, walkable surfaces' and client side's, into the space's.
#[derive(Clone)]
pub struct Arrangement {
    pub agent: Anchor,
    pub home: Anchor,
    pub workers: Anchor,
    pub audience: Vec3,
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
            audience: EYES,
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
            .field("audience", &self.audience)
            .field("placement", &self.placement.is_some())
            .field("carriage", &self.carriage)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::delegation::slot_position;

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
        // With the arrangement's middle straight above the eyes it gathers about, its arc stands the way the middle faces
        // them from: facing +X, it faces them from −X, so its workers stand towards −X.
        let overhead = Anchor {
            position: EYES + Vec3::Y,
            orientation: Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
        };
        let first = HomeArc::at(overhead, EYES, 0);
        assert!(
            first.x < -1.0
                && (Vec3::new(first.x, 0.0, first.z).length() - NEAREST_ARC).abs() < 1e-4
        );
        // With the arrangement's middle 6 m further on, the arc about the eyes at home widens to pass through it, each
        // worker at its column's own angle from it, its rows about its height spread in proportion, as its signs are
        // drawn larger.
        let further = Anchor {
            position: crate::swarm::CENTER + Vec3::new(0.0, 0.0, -6.0),
            orientation: Quat::IDENTITY,
        };
        let (moved, panicked) = place(further, None, &worker);
        assert!(!panicked);
        assert!((Vec3::new(moved.x, 0.0, moved.z).length() - 8.3).abs() < 1e-4);
        let (_, column, _) = crate::delegation::slot_lane(3);
        assert!(
            (moved.x.atan2(-moved.z) - crate::delegation::slot_angle(column)).abs() < 1e-5,
            "{moved}"
        );
        let spread = 8.3 / ARC_RADIUS;
        assert!((moved.y - (further.position.y - ROW_BELOW * spread)).abs() < 1e-4);
        let above = HomeArc::at(further, EYES, 4);
        assert!((above.y - (further.position.y + (ROW_APART - ROW_BELOW) * spread)).abs() < 1e-4);
        // Nearer than the arc's own radius, its columns stand as far apart along it as there: at the default home, from
        // 55⅔°, those past the last there in rows above.
        let near = |slot: usize| {
            let at = slot_position(slot);
            at.x.atan2(-at.z).abs().to_degrees()
        };
        assert!(
            (near(0) - 40.0 * ARC_RADIUS / 2.3).abs() < 1e-3,
            "{}",
            near(0)
        );
        assert!((near(2) - near(0) - 280.0 / 15.0 * ARC_RADIUS / 2.3).abs() < 1e-3);
        // Its column 5, past 170⅔° there, and its third row, stand on the arc 3.2 m from home, at their own angles, two
        // rows up, above and behind the band.
        assert_eq!(crate::delegation::slot_lane(18), (-1.0, 5, 0));
        let outer = slot_position(18);
        assert!((Vec3::new(outer.x, 0.0, outer.z).length() - ARC_RADIUS).abs() < 1e-4);
        assert!((near(18) - crate::delegation::slot_angle(5).to_degrees()).abs() < 1e-3);
        assert!((outer.y - (slot_position(0).y + 2.0 * ROW_APART)).abs() < 1e-4);
        assert_eq!(crate::delegation::slot_lane(32), (-1.0, 0, 2));
        let third = slot_position(32);
        assert!((Vec3::new(third.x, 0.0, third.z).length() - ARC_RADIUS).abs() < 1e-4);
        assert!((third.y - (slot_position(0).y + 2.0 * ROW_APART)).abs() < 1e-4);
        // A middle low by the floor keeps every worker above it.
        let low = Anchor {
            position: Vec3::new(0.0, 0.2, -3.0),
            orientation: Quat::IDENTITY,
        };
        assert!((HomeArc::at(low, EYES, 0).y - 0.3).abs() < 1e-5);
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
        assert_eq!(HomeArc::at(workers, EYES, 2), slot_position(2));
        // To one side, facing home, its arc still curves about home: its middle slot stands nearer home than the
        // arrangement's middle, on the way to home, as the default's does.
        let side = Pose::at([2.3, 1.4, 0.0]).facing_towards([0.0, 1.4, 0.0]);
        let side = Anchor::from_pose(&side).unwrap();
        let yaw = side.yaw();
        assert!((yaw + std::f32::consts::FRAC_PI_2).abs() < 1e-5, "{yaw}");
        let ahead = side.place(Vec3::Z);
        assert!(close(ahead, Vec3::new(1.3, 1.4, 0.0)), "{ahead}");
        for slot in 0..8 {
            let at = HomeArc::at(side, EYES, slot);
            // On the circle about home through the arrangement's middle, 2.3 m from home, as the default arc is.
            assert!(
                (Vec3::new(at.x, 0.0, at.z).length() - 2.3).abs() < 1e-3,
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
