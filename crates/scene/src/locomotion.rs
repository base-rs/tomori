// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Travel beyond the room: the locomotion profile an environment declares, its walkable surfaces and where a ray lands on them, the participant's [`Rig`], and [`Travel`], which turns the left stick and the ray into teleports, snap turns, backsteps and rising. Pure, so that it is tested without a device; the frame step applies the rig to every pose. See `design/locomotion.md`.
use glam::{Mat4, Quat, Vec2, Vec3};

use crate::{anchors::Anchor, environment::Environment, pointing::Ray};

/// How far a snap turn turns, in radians.
pub const SNAP_TURN: f32 = std::f32::consts::PI / 6.0;
/// How fast flying rises, in metres a second.
pub const ASCENT_SPEED: f32 = 0.6;
/// The furthest a landing can be from the ray's origin, horizontally, in metres.
pub const REACH: f32 = 15.0;
/// The nearest a landing can be along the ray, in metres.
pub const NEAREST: f32 = 0.25;
/// How far a backstep steps, horizontally, opposite to where the head faces, in metres.
pub const BACKSTEP: f32 = 0.6;
/// The shortest backstep worth taking where the floor ends behind the participant, in metres: less than this, it steps nowhere.
pub const SHORTEST_BACKSTEP: f32 = 0.2;
/// How finely a backstep's path is checked for floor, in metres.
const BACKSTEP_STRIDE: f32 = 0.05;
/// How near a surface's height the room's floor must be for the participant to stand on it, in metres.
const STANDING: f32 = 0.01;
/// How long the stick is held back, where the environment flies, before it rises rather than stepping back, in seconds.
pub const HOLD: f32 = 0.3;
/// How far forward the stick goes before it aims a teleport, and how far back before it steps back or rises.
const PUSH: f32 = 0.7;
/// How far sideways the stick goes before it turns.
const TURN: f32 = 0.6;
/// How near the middle the stick returns to end a gesture: a teleport aimed lands, and the next push counts.
const CENTRE: f32 = 0.2;
/// How long the view takes to come back from dark after a teleport, in seconds.
pub const BLINK: f32 = 0.2;
/// How far below the horizontal a ray must point to land anywhere, as the sine of the angle.
const DOWNWARDS: f32 = 0.02;
/// A teleport pad's radius unless it is declared with another, in metres.
pub const PAD_RADIUS: f32 = 0.5;
/// The widest a teleport pad may be, in metres: a pair with a wider pad is ignored.
pub const WIDEST_PAD: f32 = 2.0;
/// The most characters of a pad's name that are kept.
pub const LONGEST_NAME: usize = 24;
/// The most pairs of pads a profile uses: those declared after them are ignored.
pub const MOST_PAIRS: usize = 8;

/// A horizontal surface a participant may land on, in the space's scene coordinates. Made with [`Surface::rectangle`] and [`Surface::disc`], so that it can grow, as with colliders or rotated surfaces, without breaking:
///
/// ```
/// use glam::{Vec2, Vec3};
/// use tomori_scene::locomotion::{Locomotion, Surface};
///
/// let deck = Surface::rectangle(Vec3::new(0.0, 3.0, 0.0), Vec2::new(4.0, 2.0));
/// let profile = Locomotion::fly(vec![deck, Surface::disc(Vec3::ZERO, 10.0)], 12.0);
/// assert_eq!(profile.ceiling(), Some(12.0));
/// ```
///
/// A variant's fields cannot be given directly outside this crate:
///
/// ```compile_fail
/// let _ = tomori_scene::locomotion::Surface::Disc { centre: glam::Vec3::ZERO, radius: 1.0 };
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum Surface {
    /// A rectangle centred at `centre`, `half` its size along its own X and Z, turned `yaw` about the vertical from the
    /// scene's axes: aligned with them as an environment declares it, with [`Surface::rectangle`], and turned as it is
    /// carried into a space whose home is elsewhere in the environment (see [`Locomotion::carried`]).
    #[non_exhaustive]
    Rectangle { centre: Vec3, half: Vec2, yaw: f32 },
    /// A disc centred at `centre`. Made with [`Surface::disc`].
    #[non_exhaustive]
    Disc { centre: Vec3, radius: f32 },
}

impl Surface {
    /// A rectangle centred at `centre`, at its height, `size` across along X and Z.
    pub fn rectangle(centre: Vec3, size: Vec2) -> Self {
        Self::Rectangle {
            centre,
            half: size.abs() * 0.5,
            yaw: 0.0,
        }
    }

    /// A disc centred at `centre`, at its height.
    pub fn disc(centre: Vec3, radius: f32) -> Self {
        Self::Disc {
            centre,
            radius: radius.abs(),
        }
    }

    /// The surface, given in an environment's own coordinates, in a space's, where `carriage` takes them: moved and
    /// turned with it.
    pub fn carried(&self, carriage: &crate::anchors::Anchor) -> Self {
        match *self {
            Self::Rectangle { centre, half, yaw } => Self::Rectangle {
                centre: carriage.place(centre),
                half,
                yaw: yaw + carriage.yaw(),
            },
            Self::Disc { centre, radius } => Self::Disc {
                centre: carriage.place(centre),
                radius,
            },
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
            Self::Rectangle { centre, half, yaw } => {
                let offset = Vec3::new(point.x - centre.x, 0.0, point.z - centre.z);
                let local = if yaw == 0.0 {
                    offset
                } else {
                    Quat::from_rotation_y(-yaw) * offset
                };
                local.x.abs() <= half.x && local.z.abs() <= half.y
            }
            Self::Disc { centre, radius } => {
                Vec2::new(point.x - centre.x, point.z - centre.z).length() <= radius
            }
        }
    }
}

/// A teleport pad: a disc at a height, in the space's scene coordinates, which transports a participant who steps onto
/// it, or points at it and presses, to the pad it is paired with, as [`Locomotion::with_pads`] pairs them. Tomori draws
/// it, so an environment says only where it is; it may say how wide it is, which way a participant arriving on it
/// faces, and its name, which the label on the pad paired with it shows. See `design/locomotion.md`.
///
/// ```
/// use glam::{Vec2, Vec3};
/// use tomori_scene::locomotion::{Locomotion, Pad, Surface};
///
/// let floor = Surface::rectangle(Vec3::ZERO, Vec2::new(20.0, 20.0));
/// let racks = Surface::rectangle(Vec3::new(0.0, -4.5, 0.0), Vec2::new(20.0, 20.0));
/// let profile = Locomotion::fly(vec![floor, racks], 6.0).with_pads([[
///     Pad::at(Vec3::new(3.0, 0.0, -2.0)).named("Main floor"),
///     Pad::at(Vec3::new(3.0, -4.5, -2.0)).named("Racks").facing(Vec3::NEG_X),
/// ]]);
/// assert_eq!(profile.pads().count(), 1);
/// ```
#[derive(Clone, Debug, PartialEq)]
pub struct Pad {
    centre: Vec3,
    radius: f32,
    /// Which way a participant arriving on it faces, as a turn about the vertical from −Z, as [`Rig`] turns, or `None`
    /// for the way they faced.
    yaw: Option<f32>,
    name: Option<std::sync::Arc<str>>,
}

impl Pad {
    /// A pad centred at `centre`, at its height, [`PAD_RADIUS`] across, unnamed, where a participant arriving keeps
    /// facing the way they faced.
    pub fn at(centre: Vec3) -> Self {
        Self {
            centre,
            radius: PAD_RADIUS,
            yaw: None,
            name: None,
        }
    }

    /// The pad, `radius` across instead.
    pub fn with_radius(mut self, radius: f32) -> Self {
        self.radius = radius.abs();
        self
    }

    /// The pad, where a participant arriving faces `direction`, horizontally. A direction with no horizontal part
    /// leaves them facing the way they faced.
    pub fn facing(mut self, direction: Vec3) -> Self {
        self.yaw = Vec3::new(direction.x, 0.0, direction.z)
            .try_normalize()
            .map(|level| (-level.x).atan2(-level.z));
        self
    }

    /// The pad, called `name`, which the label on the pad paired with it shows, as "Racks ↓". Control characters are
    /// dropped and the name kept to its first [`LONGEST_NAME`] characters; a name left with none is no name.
    pub fn named(mut self, name: impl AsRef<str>) -> Self {
        let name: String = name
            .as_ref()
            .chars()
            .filter(|ch| !ch.is_control())
            .take(LONGEST_NAME)
            .collect();
        self.name = (!name.is_empty()).then(|| name.into());
        self
    }

    /// Where it is, at its height.
    pub fn centre(&self) -> Vec3 {
        self.centre
    }

    /// How far across it is from its centre.
    pub fn radius(&self) -> f32 {
        self.radius
    }

    /// Which way a participant arriving on it faces, as a turn about the vertical from −Z, if it says.
    pub fn yaw(&self) -> Option<f32> {
        self.yaw
    }

    /// What it is called, if anything.
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// The pad, declared in an environment's own coordinates, in a space's, where `carriage` takes them: moved and
    /// turned with it.
    pub fn carried(&self, carriage: &crate::anchors::Anchor) -> Self {
        Self {
            centre: carriage.place(self.centre),
            yaw: self.yaw.map(|yaw| yaw + carriage.yaw()),
            ..self.clone()
        }
    }

    /// Whether `point`, seen from above, is within `margin` of it.
    pub fn reaches(&self, point: Vec3, margin: f32) -> bool {
        Vec2::new(point.x - self.centre.x, point.z - self.centre.z).length() <= self.radius + margin
    }

    /// Whether a participant may be taken to it and from it: finite, no wider than [`WIDEST_PAD`], within
    /// [`crate::anchors::REACH`] of home on each axis, as the environment's anchors are, and standing on one of
    /// `surfaces`, so that whoever arrives on it stands on a floor.
    fn usable(&self, surfaces: &[Surface]) -> bool {
        self.centre.is_finite()
            && self.centre.abs().max_element() <= crate::anchors::REACH
            && self.radius > 0.0
            && self.radius <= WIDEST_PAD
            && self.yaw.is_none_or(f32::is_finite)
            && surfaces.iter().any(|surface| {
                (surface.height() - self.centre.y).abs() <= STANDING
                    && surface.contains(self.centre)
            })
    }
}

/// How an environment lets its participants travel beyond the room. Made with [`Locomotion::teleport`] and [`Locomotion::fly`], and given pairs of teleport pads with [`Locomotion::with_pads`], so that it can grow, as with a fly speed, without breaking; a variant's fields cannot be given directly outside this crate:
///
/// ```compile_fail
/// let _ = tomori_scene::locomotion::Locomotion::Teleport { surfaces: Vec::new(), pads: Vec::new() };
/// ```
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub enum Locomotion {
    /// Room scale, as before: no artificial travel.
    #[default]
    None,
    /// Teleport onto the walkable `surfaces`, and turn in snaps, and between each pair of `pads`. Made with
    /// [`Locomotion::teleport`].
    #[non_exhaustive]
    Teleport {
        surfaces: Vec<Surface>,
        pads: Vec<[Pad; 2]>,
    },
    /// Teleport and turn, and rise as well, never above `ceiling`, the highest the room's floor goes, and between each
    /// pair of `pads`. Made with [`Locomotion::fly`].
    #[non_exhaustive]
    Fly {
        surfaces: Vec<Surface>,
        ceiling: f32,
        pads: Vec<[Pad; 2]>,
    },
}

impl Locomotion {
    /// Teleport onto `surfaces`, and turn in snaps.
    pub fn teleport(surfaces: Vec<Surface>) -> Self {
        Self::Teleport {
            surfaces,
            pads: Vec::new(),
        }
    }

    /// Teleport onto `surfaces`, turn in snaps, and rise up to `ceiling`.
    pub fn fly(surfaces: Vec<Surface>, ceiling: f32) -> Self {
        Self::Fly {
            surfaces,
            ceiling,
            pads: Vec::new(),
        }
    }

    /// The profile, with `pairs` of teleport pads as well, each pad taking a participant to the other, both ways. A
    /// profile of none has no pads, since nothing travels there. Only the first [`MOST_PAIRS`] pairs whose pads may be
    /// used are, as [`Self::pads`] gives them.
    pub fn with_pads(mut self, pairs: impl IntoIterator<Item = [Pad; 2]>) -> Self {
        if let Self::Teleport { pads, .. } | Self::Fly { pads, .. } = &mut self {
            pads.extend(pairs);
        }
        self
    }

    /// The pairs of teleport pads a participant may use: the first [`MOST_PAIRS`] of those declared whose pads are
    /// both finite, no wider than [`WIDEST_PAD`], within [`crate::anchors::REACH`] of home on each axis and standing
    /// on a declared surface.
    pub fn pads(&self) -> impl Iterator<Item = &[Pad; 2]> {
        let (surfaces, pads): (&[Surface], &[[Pad; 2]]) = match self {
            Self::None => (&[], &[]),
            Self::Teleport { surfaces, pads } | Self::Fly { surfaces, pads, .. } => {
                (surfaces, pads)
            }
        };
        pads.iter()
            .filter(move |pair| pair.iter().all(|pad| pad.usable(surfaces)))
            .take(MOST_PAIRS)
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

    /// The profile, declared in an environment's own coordinates, in a space's, where `carriage` takes them, as for an
    /// environment whose participants arrive elsewhere than its origin: its surfaces moved and turned, and its ceiling
    /// raised or lowered, with it. Unchanged where the carriage leaves everything where it is. See
    /// `design/environments.md`.
    pub fn carried(&self, carriage: &crate::anchors::Anchor) -> Self {
        if *carriage == crate::anchors::Anchor::HOME {
            return self.clone();
        }
        let carry = |surfaces: &[Surface]| {
            surfaces
                .iter()
                .map(|surface| surface.carried(carriage))
                .collect()
        };
        let pairs = |pads: &[[Pad; 2]]| -> Vec<[Pad; 2]> {
            pads.iter()
                .map(|pair| pair.each_ref().map(|pad| pad.carried(carriage)))
                .collect()
        };
        match self {
            Self::None => Self::None,
            Self::Teleport { surfaces, pads } => {
                Self::teleport(carry(surfaces)).with_pads(pairs(pads))
            }
            Self::Fly {
                surfaces,
                ceiling,
                pads,
            } => Self::fly(carry(surfaces), ceiling + carriage.position.y).with_pads(pairs(pads)),
        }
    }

    pub(crate) fn surfaces(&self) -> &[Surface] {
        match self {
            Self::None => &[],
            Self::Teleport { surfaces, .. } | Self::Fly { surfaces, .. } => surfaces,
        }
    }

    /// Whether a declared surface crosses `ray`, in either direction, before `distance` along it: what hides a pad
    /// beneath a deck from above, and one on top of it from below.
    pub(crate) fn surface_crossed(&self, ray: &Ray, distance: f32) -> bool {
        if ray.direction.y.abs() < f32::EPSILON {
            return false;
        }
        self.surfaces().iter().any(|surface| {
            let crossed = (surface.height() - ray.origin.y) / ray.direction.y;
            crossed >= 0.0 && crossed < distance && surface.contains(ray.at(crossed))
        })
    }

    /// How far along `ray` it first meets a surface from above, however near or far, and where, at the surface's
    /// height.
    pub(crate) fn surface_hit(&self, ray: &Ray) -> Option<(f32, Vec3)> {
        if ray.direction.y > -DOWNWARDS {
            return None;
        }
        self.surfaces()
            .iter()
            .filter_map(|surface| {
                let distance = (surface.height() - ray.origin.y) / ray.direction.y;
                let point = ray.at(distance);
                (distance >= 0.0 && surface.contains(point))
                    .then_some((distance, Vec3::new(point.x, surface.height(), point.z)))
            })
            .min_by(|(a, _), (b, _)| a.total_cmp(b))
    }

    /// Where a backstep from `head` goes, facing `facing`, horizontal, with the room's floor at `floor`: [`BACKSTEP`] straight back, horizontally. Where the profile teleports, it steps only along the surface it stands on, at the floor's height, as far as that goes without a gap, checked every 5 cm, or nowhere when that is less than [`SHORTEST_BACKSTEP`]; where it flies, the participant may hover, so it goes the whole way. `None` for a profile of none, or where it steps nowhere.
    pub fn backstep(&self, head: Vec3, facing: Vec3, floor: f32) -> Option<Vec3> {
        let back = -Vec3::new(facing.x, 0.0, facing.z).try_normalize()?;
        let distance = match self {
            Self::None => return None,
            Self::Fly { .. } => BACKSTEP,
            Self::Teleport { surfaces, .. } => {
                let walkable = |distance: f32| {
                    let point = head + back * distance;
                    surfaces.iter().any(|surface| {
                        (surface.height() - floor).abs() <= STANDING && surface.contains(point)
                    })
                };
                let strides = (BACKSTEP / BACKSTEP_STRIDE).round() as u32;
                (1..=strides)
                    .map(|stride| stride as f32 * BACKSTEP_STRIDE)
                    .take_while(|distance| walkable(*distance))
                    .last()
                    .filter(|distance| *distance >= SHORTEST_BACKSTEP - f32::EPSILON)?
            }
        };
        Some(back * distance)
    }

    /// Where `ray` lands: on the first surface it meets from above, if that is at least [`NEAREST`] along it and at most [`REACH`] from its origin horizontally, or `None`. A surface nearer than that still hides those beneath it.
    pub fn landing(&self, ray: &Ray) -> Option<Vec3> {
        let (distance, point) = self.surface_hit(ray)?;
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

    /// Move the room by `offset`, horizontally, keeping its floor's height.
    pub fn step(&mut self, offset: Vec3) {
        self.translation += Vec3::new(offset.x, 0.0, offset.z);
    }

    /// Raise the room by `distance`, never above `ceiling`, nor lower than it is.
    pub fn ascend(&mut self, distance: f32, ceiling: f32) {
        self.translation.y = (self.translation.y + distance).min(ceiling.max(self.translation.y));
    }

    /// `anchor`, in the room, in the scene: moved and turned with the room. At home, it is `anchor` itself.
    pub fn place(&self, anchor: Anchor) -> Anchor {
        if *self == Self::default() {
            return anchor;
        }
        let turn = Quat::from_rotation_y(self.yaw);
        Anchor {
            position: self.translation + turn * anchor.position,
            orientation: turn * anchor.orientation,
        }
    }

    /// `anchor`, in the scene, in the room: where [`Self::place`] would take it from.
    pub fn unplace(&self, anchor: Anchor) -> Anchor {
        if *self == Self::default() {
            return anchor;
        }
        let back = Quat::from_rotation_y(-self.yaw);
        Anchor {
            position: back * (anchor.position - self.translation),
            orientation: back * anchor.orientation,
        }
    }
}

/// Which way a head turned by `orientation` faces, horizontally: its forward, levelled, or, looking nearly straight up or down, the way the top of the head leans. `None` for an orientation that is nonsense.
pub fn facing(orientation: Quat) -> Option<Vec3> {
    if !orientation.is_finite() || orientation.length() < 0.5 {
        return None;
    }
    let orientation = orientation.normalize();
    let level = |direction: Vec3| {
        let level = Vec3::new(direction.x, 0.0, direction.z);
        (level.length() > 0.1).then(|| level.normalize())
    };
    let forward = orientation * Vec3::NEG_Z;
    let up = orientation * Vec3::Y;
    // Looking down, the top of the head leans forward; looking up, back.
    level(forward).or_else(|| level(up * -forward.y.signum()))
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
    /// Which way the head faces, in the scene, from which a backstep steps straight back.
    pub facing: Option<Vec3>,
    /// Nothing else takes the stick and the ray: in a space, focused, neither the hand menu nor the transcript showing, and no card carried. A panel carried does not stop travel, which carries it along.
    pub active: bool,
    /// The teleport pad the participant chose with the ray this frame, if any, as `crate::pads` keys it among the
    /// profile's pairs.
    pub chosen: Option<crate::pads::Key>,
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
    /// It stepped back this frame.
    pub stepped: bool,
    /// A teleport pad took the participant to the pad paired with it this frame.
    pub transported: bool,
    /// How dark the view is, from 1, as a teleport lands or a backstep steps, back to 0 over [`BLINK`] seconds.
    pub blink: f32,
}

/// What the left stick is doing: each push is one gesture, owned by the axis that passed its threshold first, until the stick returns to the middle.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Gesture {
    /// At the middle: the next push starts a gesture.
    #[default]
    Ready,
    /// Forward came first: a teleport is aimed, and lands when the stick returns to the middle.
    Aiming,
    /// Back came first: it steps back when the stick returns to the middle, unless, where the environment flies, it has been held for [`HOLD`] seconds, as long as it has been held.
    Back(f32),
    /// Back was held, where the environment flies: it rises while the stick is held back.
    Rising,
    /// The push has done what it does, as a turn does, or was cancelled: nothing more until the middle.
    Spent,
}

/// The participant's rig, and the gesture the left stick is making, between frames.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Travel {
    rig: Rig,
    /// The space and environment the rig belongs to.
    place: Option<(String, Environment)>,
    gesture: Gesture,
    landing: Option<Vec3>,
    /// How dark the view is after a teleport or a backstep.
    blink: f32,
    /// Which teleport pad the participant stands on, for how long, and whether it is disarmed.
    stepping: crate::pads::Stepping,
}

impl Travel {
    /// Where the participant's room is in the scene, as it was last settled.
    #[cfg(test)]
    pub fn rig(&self) -> Rig {
        self.rig
    }

    /// Drop a teleport being aimed, as on losing focus; the next push needs the stick back at the middle first.
    pub fn cancel(&mut self) {
        self.gesture = Gesture::Spent;
        self.landing = None;
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
        let dt = if input.dt.is_finite() {
            input.dt.max(0.0)
        } else {
            0.0
        };
        self.blink = (self.blink - dt / BLINK).max(0.0);
        if !input.locomotion.travels() || !input.active {
            self.cancel();
            self.stepping.pause();
            return Outcome {
                blink: self.blink,
                ..Outcome::default()
            };
        }
        // A teleport pad stepped on for long enough, or chosen with the ray, takes the participant to the pad paired with
        // it, as the frame's poses were placed, before the stick moves anything, and nothing else moves them this frame.
        let pairs: Vec<&[Pad; 2]> = input.locomotion.pads().collect();
        let stepped = self
            .stepping
            .update(&pairs, input.head, self.rig.floor(), dt);
        if let (Some(key), Some(head)) = (input.chosen.or(stepped), input.head)
            && let Some(partner) = crate::pads::lookup(&pairs, key.partner())
        {
            self.rig.teleport(partner.centre(), head);
            if let (Some(yaw), Some(facing)) = (partner.yaw(), input.facing) {
                let faced = (-facing.x).atan2(-facing.z);
                self.rig.turn(yaw - faced, partner.centre().with_y(head.y));
            }
            self.stepping.arrive(key.partner());
            self.cancel();
            self.blink = 1.0;
            return Outcome {
                transported: true,
                blink: self.blink,
                ..Outcome::default()
            };
        }
        // A stick that reads nonsense is at the middle, but lands nothing.
        let stick = if input.stick.is_finite() {
            input.stick
        } else {
            self.landing = None;
            Vec2::ZERO
        };
        let mut outcome = Outcome::default();
        if stick.length() < CENTRE {
            // Back at the middle: a teleport aimed lands where it aimed, if the ray is still there. Losing it, as a controller does that stops being tracked, cancels.
            if let (Gesture::Aiming, Some(landing), Some(head), Some(_)) =
                (self.gesture, self.landing, input.head, input.ray)
            {
                self.rig.teleport(landing, head);
                self.blink = 1.0;
                outcome.teleported = true;
            }
            // A push back steps back, where there is floor to step onto.
            if let (Gesture::Back(_), Some(head), Some(facing)) =
                (self.gesture, input.head, input.facing)
                && let Some(offset) = input.locomotion.backstep(head, facing, self.rig.floor())
            {
                self.rig.step(offset);
                self.blink = 1.0;
                outcome.stepped = true;
            }
            self.gesture = Gesture::Ready;
            self.landing = None;
        } else if self.gesture == Gesture::Ready {
            // The first axis pushed past its threshold owns the push, so that easing off, or rolling the thumb to another side, neither lands nor turns.
            let (across, along) = (stick.x.abs(), stick.y);
            if along > PUSH && along >= across {
                self.gesture = Gesture::Aiming;
            } else if across > TURN && across > along.abs() {
                self.gesture = Gesture::Spent;
                if let Some(head) = input.head {
                    self.rig.turn(-stick.x.signum() * SNAP_TURN, head);
                    outcome.turned = true;
                }
            } else if -along > PUSH && -along >= across {
                self.gesture = Gesture::Back(0.0);
            }
        }
        match self.gesture {
            // While aiming, the ray finds where it would land; losing it cancels.
            Gesture::Aiming => match input.ray {
                Some(ray) => self.landing = input.locomotion.landing(&ray),
                None => self.cancel(),
            },
            // Held back long enough, where the environment flies, it rises instead of stepping back, from the moment it has been held that long.
            Gesture::Back(held) => {
                let now = held + dt;
                self.gesture = Gesture::Back(now);
                if let Some(ceiling) = input.locomotion.ceiling()
                    && now > HOLD
                {
                    self.gesture = Gesture::Rising;
                    if stick.y < -CENTRE {
                        self.rig.ascend(ASCENT_SPEED * (now - HOLD), ceiling);
                    }
                }
            }
            // Rising, while the stick is still held back, never above the ceiling.
            Gesture::Rising => {
                if let Some(ceiling) = input.locomotion.ceiling()
                    && stick.y < -CENTRE
                {
                    self.rig.ascend(ASCENT_SPEED * dt, ceiling);
                }
            }
            Gesture::Ready | Gesture::Spent => {}
        }
        outcome.aiming = self.gesture == Gesture::Aiming;
        outcome.landing = self.landing;
        outcome.blink = self.blink;
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
            facing: Some(Vec3::NEG_Z),
            active: true,
            chosen: None,
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
    fn back_rises_only_where_the_environment_flies_and_once_held() {
        let floors = floors();
        let mut travel = Travel::default();
        travel.update(input(&floors, Vec2::NEG_Y, None));
        travel.update(input(&floors, Vec2::NEG_Y, None));
        assert_eq!(travel.rig().floor(), 0.0);
        travel.update(input(&floors, Vec2::ZERO, None));
        let fly = Locomotion::fly(Vec::new(), 1.0);
        let mut travel = Travel::default();
        // Held past the hold, it rises from then on: 0.2 s of the first half second.
        travel.update(input(&fly, Vec2::NEG_Y, None));
        assert!((travel.rig().floor() - 0.12).abs() < 1e-5);
        travel.update(input(&fly, Vec2::NEG_Y, None));
        assert!((travel.rig().floor() - 0.42).abs() < 1e-5);
        // A frame without a sensible length, or a push more sideways, rises nothing.
        let nonsense = Input {
            dt: f32::NAN,
            ..input(&fly, Vec2::NEG_Y, None)
        };
        travel.update(nonsense);
        assert!((travel.rig().floor() - 0.42).abs() < 1e-5);
        travel.update(input(&fly, Vec2::ZERO, None));
        assert!(
            travel
                .update(input(&fly, Vec2::new(0.9, -0.7), None))
                .turned
        );
        assert!((travel.rig().floor() - 0.42).abs() < 1e-5);
        travel.update(input(&fly, Vec2::ZERO, None));
        for _ in 0..10 {
            travel.update(input(&fly, Vec2::NEG_Y, None));
        }
        assert_eq!(travel.rig().floor(), 1.0);
        // Released after rising, it steps nowhere.
        let before = travel.rig();
        assert!(!travel.update(input(&fly, Vec2::ZERO, None)).stepped);
        assert_eq!(travel.rig(), before);
    }

    #[test]
    fn heads_face_the_way_they_look_levelled() {
        assert!(close(facing(Quat::IDENTITY).unwrap(), Vec3::NEG_Z));
        let left = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
        assert!(close(facing(left).unwrap(), Vec3::NEG_X));
        // Looking down, or up, a little, it is still forward.
        let down = Quat::from_rotation_x(-1.2);
        assert!(close(facing(down).unwrap(), Vec3::NEG_Z));
        // Straight down or up, the top of the head says which way is forward.
        let floor = Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2);
        assert!(close(facing(floor).unwrap(), Vec3::NEG_Z));
        let sky = left * Quat::from_rotation_x(std::f32::consts::FRAC_PI_2);
        assert!(close(facing(sky).unwrap(), Vec3::NEG_X));
        // Nonsense faces nowhere.
        assert_eq!(facing(Quat::from_xyzw(0.0, 0.0, 0.0, 0.0)), None);
        assert_eq!(facing(Quat::from_xyzw(f32::NAN, 0.0, 0.0, 1.0)), None);
    }

    #[test]
    fn a_backstep_goes_straight_back_as_far_as_the_floor_goes() {
        let head = Vec3::new(0.0, 1.7, 0.0);
        // A small floor, its back edge 0.42 m behind the head: it steps as far as the floor goes, in 5 cm strides.
        let small = Locomotion::teleport(vec![Surface::rectangle(
            Vec3::new(0.0, 0.0, -0.79),
            Vec2::new(2.0, 2.42),
        )]);
        let step = small.backstep(head, Vec3::NEG_Z, 0.0).unwrap();
        assert!(close(step, Vec3::new(0.0, 0.0, 0.4)), "{step}");
        // Facing anywhere, it steps away from it, horizontally, however the face tilts.
        let wide = Locomotion::teleport(vec![Surface::disc(Vec3::ZERO, 5.0)]);
        let step = wide.backstep(head, Vec3::new(1.0, -3.0, 0.0), 0.0).unwrap();
        assert!(close(step, Vec3::new(-BACKSTEP, 0.0, 0.0)), "{step}");
        // Only along the surface stood on: a ledge above the floor does not stop a step on the floor, as nothing
        // collides yet, and from the ledge, the floor below is not stepped down onto.
        let ledged = Locomotion::teleport(vec![
            Surface::disc(Vec3::ZERO, 5.0),
            Surface::rectangle(Vec3::new(0.0, 0.5, 0.6), Vec2::new(2.0, 0.4)),
        ]);
        let step = ledged.backstep(head, Vec3::NEG_Z, 0.0).unwrap();
        assert!(close(step, Vec3::new(0.0, 0.0, BACKSTEP)), "{step}");
        let on_ledge = Vec3::new(0.0, 2.2, 0.45);
        assert_eq!(ledged.backstep(on_ledge, Vec3::Z, 0.5), None);
        // Too close to the edge, it steps nowhere: 0.15 m is less than is worth a step.
        let near = Locomotion::teleport(vec![Surface::rectangle(
            Vec3::new(0.0, 0.0, -0.84),
            Vec2::new(2.0, 2.0),
        )]);
        assert_eq!(near.backstep(head, Vec3::NEG_Z, 0.0), None);
        // Not over a gap: the floor beyond one is not reached.
        let gap = Locomotion::teleport(vec![
            Surface::rectangle(Vec3::new(0.0, 0.0, -1.0), Vec2::new(2.0, 2.64)),
            Surface::rectangle(Vec3::new(0.0, 0.0, 1.5), Vec2::new(2.0, 2.0)),
        ]);
        let step = gap.backstep(head, Vec3::NEG_Z, 0.0).unwrap();
        assert!(close(step, Vec3::new(0.0, 0.0, 0.3)), "{step}");
        // Flying, the whole way, floor or not; with no travel, or no way it faces, nowhere.
        let fly = Locomotion::fly(Vec::new(), 10.0);
        let step = fly.backstep(head, Vec3::NEG_Z, 3.0).unwrap();
        assert!(close(step, Vec3::new(0.0, 0.0, BACKSTEP)));
        assert_eq!(Locomotion::None.backstep(head, Vec3::NEG_Z, 0.0), None);
        assert_eq!(wide.backstep(head, Vec3::Y, 0.0), None);
    }

    #[test]
    fn back_steps_back_once_as_the_stick_returns_to_the_middle() {
        let floors = floors();
        let mut travel = Travel::default();
        // Pushed back, nothing moves until the stick returns, however long it is held.
        for _ in 0..4 {
            let outcome = travel.update(input(&floors, Vec2::new(0.1, -0.9), None));
            assert!(!outcome.stepped && !outcome.turned && !outcome.aiming);
        }
        // Eased off, or rolled to the side or forward, it still steps back, and only on the middle.
        for rolled in [
            Vec2::new(0.0, -0.3),
            Vec2::new(0.9, 0.0),
            Vec2::new(0.0, 0.9),
        ] {
            let outcome = travel.update(input(&floors, rolled, None));
            assert!(
                !outcome.stepped && !outcome.turned && !outcome.aiming,
                "{rolled}"
            );
        }
        assert_eq!(travel.rig(), Rig::default());
        let outcome = travel.update(input(&floors, Vec2::ZERO, None));
        assert!(outcome.stepped && !outcome.teleported);
        assert_eq!(outcome.blink, 1.0, "it blinks as a teleport does");
        assert!(close(
            travel.rig().matrix().transform_point3(HEAD),
            HEAD + Vec3::Z * BACKSTEP
        ));
        assert_eq!(travel.rig().floor(), 0.0);
        assert!(!travel.update(input(&floors, Vec2::ZERO, None)).stepped);
        // Short of the threshold, or sideways first, it steps nowhere.
        travel.update(input(&floors, Vec2::new(0.0, -0.65), None));
        assert!(!travel.update(input(&floors, Vec2::ZERO, None)).stepped);
        travel.update(input(&floors, Vec2::new(0.9, -0.5), None));
        travel.update(input(&floors, Vec2::new(0.0, -0.9), None));
        assert!(!travel.update(input(&floors, Vec2::ZERO, None)).stepped);
        // Facing left, after the turn, it steps to the right; without a head or a way it faces, nowhere.
        let before = travel.rig().matrix().transform_point3(HEAD);
        let left = Input {
            facing: Some(Vec3::NEG_X),
            ..input(&floors, Vec2::NEG_Y, None)
        };
        travel.update(left);
        travel.update(Input {
            stick: Vec2::ZERO,
            ..left
        });
        assert!(close(
            travel.rig().matrix().transform_point3(HEAD),
            before + Vec3::X * BACKSTEP
        ));
        for lacking in [
            Input {
                head: None,
                ..input(&floors, Vec2::ZERO, None)
            },
            Input {
                facing: None,
                ..input(&floors, Vec2::ZERO, None)
            },
        ] {
            travel.update(input(&floors, Vec2::NEG_Y, None));
            assert!(!travel.update(lacking).stepped);
        }
        // Cancelled, as when the hand menu shows, or where the floor ends right behind, it steps nowhere.
        travel.update(input(&floors, Vec2::NEG_Y, None));
        travel.cancel();
        assert!(!travel.update(input(&floors, Vec2::ZERO, None)).stepped);
        let edge = Locomotion::teleport(vec![Surface::disc(Vec3::new(0.3, 0.0, -0.3), 0.15)]);
        let mut travel = Travel::default();
        travel.update(input(&edge, Vec2::NEG_Y, None));
        let outcome = travel.update(input(&edge, Vec2::ZERO, None));
        assert!(!outcome.stepped);
        assert_eq!(outcome.blink, 0.0, "nowhere to step, it does not blink");
        // With no travel, nowhere.
        let none = Locomotion::None;
        travel.update(input(&none, Vec2::NEG_Y, None));
        assert_eq!(
            travel.update(input(&none, Vec2::ZERO, None)),
            Outcome::default()
        );
    }

    #[test]
    fn where_the_environment_flies_a_tap_back_steps_back_and_a_hold_rises() {
        let fly = Locomotion::fly(Vec::new(), 10.0);
        let mut travel = Travel::default();
        let tap = |stick: Vec2| Input {
            dt: HOLD / 3.0,
            ..input(&fly, stick, None)
        };
        travel.update(tap(Vec2::NEG_Y));
        travel.update(tap(Vec2::NEG_Y));
        let outcome = travel.update(tap(Vec2::ZERO));
        assert!(outcome.stepped);
        assert_eq!(travel.rig().floor(), 0.0);
        assert!(close(
            travel.rig().matrix().transform_point3(HEAD),
            HEAD + Vec3::Z * BACKSTEP
        ));
        // Held past the hold, it rises rather than stepping back.
        for _ in 0..4 {
            assert!(!travel.update(tap(Vec2::NEG_Y)).stepped);
        }
        assert!((travel.rig().floor() - ASCENT_SPEED * HOLD / 3.0).abs() < 1e-5);
        assert!(!travel.update(tap(Vec2::ZERO)).stepped);
        // Rolled sideways before the hold is up, it rises nothing once it is, and still steps nowhere.
        let floor = travel.rig().floor();
        travel.update(tap(Vec2::NEG_Y));
        for _ in 0..3 {
            assert!(!travel.update(tap(Vec2::new(0.5, -0.1))).turned);
        }
        assert_eq!(travel.rig().floor(), floor);
        assert!(!travel.update(tap(Vec2::ZERO)).stepped);
    }

    /// Aim a teleport at the floor 4 m ahead, as the reviewer's cases start.
    fn aimed(floors: &Locomotion, travel: &mut Travel) -> Option<Ray> {
        let aim = Some(ray(Vec3::new(0.3, 1.2, -0.4), Vec3::new(0.0, 0.0, -4.0)));
        let outcome = travel.update(input(floors, Vec2::new(0.0, 0.9), aim));
        assert!(outcome.aiming && outcome.landing.is_some());
        aim
    }

    #[test]
    fn easing_off_while_aiming_does_not_land_until_the_stick_is_back_at_the_middle() {
        let floors = floors();
        let mut travel = Travel::default();
        let aim = aimed(&floors, &mut travel);
        // Eased from 0.9 to 0.55, below where a push starts, and further, it still aims.
        for eased in [0.55, 0.3, 0.21] {
            let outcome = travel.update(input(&floors, Vec2::new(0.0, eased), aim));
            assert!(outcome.aiming && !outcome.teleported, "{eased}");
        }
        assert_eq!(travel.rig(), Rig::default());
        // Back at the middle, it lands, once.
        let outcome = travel.update(input(&floors, Vec2::new(0.1, 0.1), aim));
        assert!(outcome.teleported && !outcome.aiming);
        assert!(close(
            travel.rig().matrix().transform_point3(HEAD),
            Vec3::new(0.0, HEAD.y, -4.0)
        ));
        assert!(!travel.update(input(&floors, Vec2::ZERO, aim)).teleported);
    }

    #[test]
    fn rolling_the_thumb_sideways_while_aiming_neither_lands_nor_turns() {
        let floors = floors();
        let mut travel = Travel::default();
        let aim = aimed(&floors, &mut travel);
        // The reviewer's roll, from forward to (0.8, 0.5), then all the way to the side and back.
        for rolled in [
            Vec2::new(0.8, 0.5),
            Vec2::new(0.95, 0.0),
            Vec2::new(-0.9, 0.1),
            Vec2::new(0.0, -0.9),
        ] {
            let outcome = travel.update(input(&floors, rolled, aim));
            assert!(outcome.aiming, "{rolled}");
            assert!(!outcome.teleported && !outcome.turned, "{rolled}");
        }
        assert_eq!(travel.rig(), Rig::default());
        let outcome = travel.update(input(&floors, Vec2::ZERO, aim));
        assert!(outcome.teleported && !outcome.turned);
        assert_eq!(travel.rig().yaw, 0.0);
    }

    #[test]
    fn whichever_axis_passes_its_threshold_first_owns_the_push_until_the_middle() {
        let floors = floors();
        let mut travel = Travel::default();
        let aim = Some(ray(HEAD, Vec3::new(0.0, 0.0, -3.0)));
        // Short of each threshold, nothing starts: forward needs 0.7, sideways 0.6.
        for short in [
            Vec2::new(0.0, 0.65),
            Vec2::new(0.55, 0.0),
            Vec2::new(-0.55, 0.3),
        ] {
            assert_eq!(
                travel.update(input(&floors, short, aim)),
                Outcome::default(),
                "{short}"
            );
        }
        // Sideways first turns, and rolling forward then aims nothing.
        assert!(
            travel
                .update(input(&floors, Vec2::new(0.9, 0.1), aim))
                .turned
        );
        for rolled in [Vec2::new(0.5, 0.8), Vec2::new(0.0, 0.95)] {
            let outcome = travel.update(input(&floors, rolled, aim));
            assert!(!outcome.aiming && !outcome.turned, "{rolled}");
        }
        // Part of the way back is not the middle: neither a turn nor an aim starts.
        travel.update(input(&floors, Vec2::new(0.25, 0.0), aim));
        assert!(
            !travel
                .update(input(&floors, Vec2::new(0.9, 0.0), aim))
                .turned
        );
        travel.update(input(&floors, Vec2::new(0.15, 0.15), aim));
        assert!(
            !travel
                .update(input(&floors, Vec2::new(0.0, 0.9), aim))
                .aiming
        );
        // From the middle, the next push counts, whichever way it goes.
        travel.update(input(&floors, Vec2::new(0.1, -0.1), aim));
        assert!(
            travel
                .update(input(&floors, Vec2::new(0.0, 0.9), aim))
                .aiming
        );
        // Forward without a ray owns the push too, and aims nothing.
        travel.cancel();
        travel.update(input(&floors, Vec2::ZERO, None));
        let outcome = travel.update(input(&floors, Vec2::Y, None));
        assert!(!outcome.aiming);
        assert!(
            !travel
                .update(input(&floors, Vec2::new(0.9, 0.0), aim))
                .turned
        );
        assert!(!travel.update(input(&floors, Vec2::ZERO, aim)).teleported);
        // A stick that reads nonsense while aiming lands nothing.
        travel.update(input(&floors, Vec2::Y, aim));
        assert!(!travel.update(input(&floors, Vec2::NAN, aim)).teleported);
        // All that moved the rig was the one turn.
        assert!((travel.rig().yaw + SNAP_TURN).abs() < 1e-5);
        assert!(close(travel.rig().matrix().transform_point3(HEAD), HEAD));
    }

    #[test]
    fn rising_owns_its_push_and_stops_as_the_stick_rolls_away() {
        let fly = Locomotion::fly(vec![Surface::disc(Vec3::ZERO, 20.0)], 10.0);
        let mut travel = Travel::default();
        let aim = Some(ray(HEAD, Vec3::new(0.0, 0.0, -3.0)));
        // Back, short of its threshold, rises nothing; past it, held, it rises.
        travel.update(input(&fly, Vec2::new(0.0, -0.65), aim));
        assert_eq!(travel.rig().floor(), 0.0);
        travel.update(input(&fly, Vec2::ZERO, aim));
        travel.update(input(&fly, Vec2::NEG_Y, aim));
        assert!((travel.rig().floor() - 0.12).abs() < 1e-5);
        // Eased off, still held back, it keeps rising.
        travel.update(input(&fly, Vec2::new(0.0, -0.4), aim));
        assert!((travel.rig().floor() - 0.42).abs() < 1e-5);
        // Rolled to the side or forward, it neither turns, aims nor rises.
        for rolled in [Vec2::new(0.9, -0.1), Vec2::new(0.1, 0.9)] {
            let outcome = travel.update(input(&fly, rolled, aim));
            assert!(!outcome.turned && !outcome.aiming, "{rolled}");
        }
        assert!((travel.rig().floor() - 0.42).abs() < 1e-5);
        let outcome = travel.update(input(&fly, Vec2::ZERO, aim));
        assert!(!outcome.teleported && !outcome.stepped);
    }

    #[test]
    fn a_teleport_darkens_the_view_and_it_comes_back_over_a_blink() {
        let floors = floors();
        let mut travel = Travel::default();
        let aim = aimed(&floors, &mut travel);
        let short = |stick: Vec2, active: bool| Input {
            dt: BLINK / 4.0,
            active,
            ..input(&floors, stick, aim)
        };
        assert_eq!(travel.update(short(Vec2::Y, true)).blink, 0.0);
        let landed = travel.update(short(Vec2::ZERO, true));
        assert!(landed.teleported);
        assert_eq!(landed.blink, 1.0);
        assert!((travel.update(short(Vec2::ZERO, true)).blink - 0.75).abs() < 1e-5);
        // It comes back while travel is inactive too, as while the hand menu shows, and nonsense time stands still.
        assert!((travel.update(short(Vec2::ZERO, false)).blink - 0.5).abs() < 1e-5);
        let nonsense = Input {
            dt: f32::NAN,
            ..short(Vec2::ZERO, true)
        };
        assert!((travel.update(nonsense).blink - 0.5).abs() < 1e-5);
        for _ in 0..3 {
            travel.update(short(Vec2::ZERO, true));
        }
        assert_eq!(travel.update(short(Vec2::ZERO, true)).blink, 0.0);
        // Turning does not darken it.
        assert_eq!(travel.update(short(Vec2::X, true)).blink, 0.0);
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
            place: Some(("space-1", crate::test_support::SKY)),
            ..input(&floors, Vec2::ZERO, None)
        });
        assert_eq!(travel.rig(), Rig::default());
        // No travel in the same environment, and nothing moves then.
        turn(&mut travel, input(&floors, Vec2::X, None));
        let none = Locomotion::None;
        travel.update(Input {
            place: Some(("space-1", crate::test_support::SKY)),
            ..input(&none, Vec2::X, None)
        });
        assert_eq!(travel.rig(), Rig::default());
        assert_eq!(
            travel.update(Input {
                place: Some(("space-1", crate::test_support::SKY)),
                ..input(&none, Vec2::X, None)
            }),
            Outcome::default()
        );
    }

    #[test]
    fn a_profile_declared_in_the_environment_s_coordinates_is_carried_into_the_space_s() {
        // A deck 4 m by 1 m along X at the environment's origin, and participants arriving 8 m along +X, raised 0.5 m,
        // facing its origin: in the space, the deck stands 8 m ahead, 0.5 m lower, and runs along Z.
        let deck = Locomotion::fly(
            vec![Surface::rectangle(
                Vec3::new(0.0, 0.5, 0.0),
                Vec2::new(4.0, 1.0),
            )],
            6.0,
        );
        let street = tomori_protocol::Pose::at([8.0, 0.5, 0.0]).facing_towards([0.0, 0.5, 0.0]);
        let carriage = crate::anchors::Anchor::carriage(&street).unwrap();
        let carried = deck.carried(&carriage);
        assert_eq!(carried.ceiling(), Some(5.5));
        let surface = carried.surfaces()[0];
        assert!(surface.height().abs() < 1e-5);
        assert!(surface.contains(Vec3::new(0.0, 0.0, -9.9)));
        assert!(surface.contains(Vec3::new(0.4, 0.0, -8.0)));
        assert!(!surface.contains(Vec3::new(1.0, 0.0, -8.0)));
        // A ray from home lands on it there.
        let landing = carried
            .landing(&ray(Vec3::new(0.0, 1.6, 0.0), Vec3::new(0.0, 0.0, -7.0)))
            .expect("lands");
        assert!(
            landing.distance(Vec3::new(0.0, 0.0, -7.0)) < 1e-3,
            "{landing}"
        );
        // Unmoved, it is as declared; and none stays none.
        assert_eq!(deck.carried(&crate::anchors::Anchor::HOME), deck);
        assert_eq!(Locomotion::None.carried(&carriage), Locomotion::None);
        let discs = Locomotion::teleport(vec![Surface::disc(Vec3::ZERO, 1.0)]).carried(&carriage);
        assert!(discs.surfaces()[0].contains(Vec3::new(0.0, 0.0, -8.5)));
    }

    /// A main floor and racks 4.5 m below it, with a pad on each, the racks' facing −X.
    fn storeys() -> Locomotion {
        Locomotion::teleport(vec![
            Surface::rectangle(Vec3::ZERO, Vec2::new(20.0, 20.0)),
            Surface::rectangle(Vec3::new(0.0, -4.5, 0.0), Vec2::new(20.0, 20.0)),
        ])
        .with_pads([[
            Pad::at(Vec3::new(0.3, 0.0, -0.2)).named("Main floor"),
            Pad::at(Vec3::new(5.0, -4.5, 1.0))
                .named("Racks")
                .facing(Vec3::new(-2.0, 7.0, 0.0)),
        ]])
    }

    #[test]
    fn pads_are_declared_where_they_are_and_may_say_more() {
        let pad = Pad::at(Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(pad.centre(), Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(pad.radius(), PAD_RADIUS);
        assert_eq!((pad.yaw(), pad.name()), (None, None));
        let pad = pad
            .with_radius(-0.8)
            .named("Deck")
            .facing(Vec3::new(1.0, -5.0, 0.0));
        assert_eq!(pad.radius(), 0.8);
        assert_eq!(pad.name(), Some("Deck"));
        // Facing +X, a quarter turn to the right of −Z, as the rig turns.
        let yaw = pad.yaw().unwrap();
        assert!(close(Quat::from_rotation_y(yaw) * Vec3::NEG_Z, Vec3::X));
        // Straight up or down faces the way they faced.
        assert_eq!(pad.clone().facing(Vec3::Y).yaw(), None);
        // Within its radius and a margin, seen from above.
        assert!(pad.reaches(Vec3::new(1.7, -9.0, 3.3), 0.0));
        assert!(!pad.reaches(Vec3::new(1.9, 2.0, 3.0), 0.0));
        assert!(pad.reaches(Vec3::new(1.9, 2.0, 3.0), 0.15));
    }

    #[test]
    fn only_pads_that_stand_on_a_surface_within_bounds_are_used() {
        let floor = || vec![Surface::disc(Vec3::ZERO, 30.0)];
        let pair = |a: Pad, b: Pad| Locomotion::teleport(floor()).with_pads([[a, b]]);
        let good = || Pad::at(Vec3::new(2.0, 0.0, 0.0));
        assert_eq!(pair(good(), good()).pads().count(), 1);
        // Off every surface, above one, too wide, beyond 20 m of home, nonsense: the pair is not used.
        for bad in [
            Pad::at(Vec3::new(2.0, 0.5, 0.0)),
            Pad::at(Vec3::new(2.0, 0.0, 0.0)).with_radius(2.5),
            Pad::at(Vec3::new(2.0, 0.0, 0.0)).with_radius(0.0),
            Pad::at(Vec3::new(20.5, 0.0, 0.0)),
            Pad::at(Vec3::new(f32::NAN, 0.0, 0.0)),
            Pad {
                yaw: Some(f32::INFINITY),
                ..good()
            },
        ] {
            assert_eq!(pair(good(), bad.clone()).pads().count(), 0, "{bad:?}");
            assert_eq!(pair(bad.clone(), good()).pads().count(), 0, "{bad:?}");
        }
        // Flying, as teleporting; no more than eight pairs; and a profile of none has none.
        let many = Locomotion::fly(floor(), 5.0).with_pads((0..10).map(|_| [good(), good()]));
        assert_eq!(many.pads().count(), MOST_PAIRS);
        assert_eq!(
            Locomotion::None.with_pads([[good(), good()]]),
            Locomotion::None
        );
        assert_eq!(Locomotion::None.pads().count(), 0);
    }

    #[test]
    fn pads_are_carried_into_the_space_with_the_profile() {
        let street = tomori_protocol::Pose::at([8.0, 0.5, 0.0]).facing_towards([0.0, 0.5, 0.0]);
        let carriage = crate::anchors::Anchor::carriage(&street).unwrap();
        for profile in [storeys(), Locomotion::fly(Vec::new(), 1.0)] {
            let declared: Vec<_> = profile.pads().cloned().collect();
            let carried = profile.carried(&carriage);
            for (pair, declared) in carried.pads().zip(&declared) {
                for (pad, declared) in pair.iter().zip(declared) {
                    assert!(close(pad.centre(), carriage.place(declared.centre())));
                    assert_eq!(pad.name(), declared.name());
                }
            }
        }
        let carried = storeys().carried(&carriage);
        let [floor, racks] = carried.pads().next().unwrap();
        assert!(close(floor.centre(), Vec3::new(0.2, -0.5, -7.7)));
        assert_eq!(floor.yaw(), None);
        // Facing −X in the environment, the way participants arrive facing, it faces −Z in the space.
        let facing = Quat::from_rotation_y(racks.yaw().unwrap()) * Vec3::NEG_Z;
        assert!(close(facing, Vec3::NEG_Z), "{facing}");
    }

    #[test]
    fn standing_on_a_pad_takes_the_participant_to_its_partner_once_and_back_once_stepped_off() {
        let storeys = storeys();
        let mut travel = Travel::default();
        let short = |head: Vec3| Input {
            head: Some(head),
            dt: 0.125,
            ..input(&storeys, Vec2::ZERO, None)
        };
        let beside = HEAD + Vec3::X * 2.0;
        // Found on it first, as on arriving in the space, the pad is disarmed until stepped off.
        for _ in 0..5 {
            assert!(!travel.update(short(HEAD)).transported);
        }
        travel.update(short(beside));
        // Stood on for 0.4 s, it takes them; walking across it does not.
        travel.update(short(HEAD));
        travel.update(short(beside));
        travel.update(short(HEAD));
        for _ in 0..3 {
            assert!(!travel.update(short(HEAD)).transported);
        }
        let outcome = travel.update(short(HEAD));
        assert!(outcome.transported && !outcome.teleported);
        assert_eq!(outcome.blink, 1.0, "it blinks as a teleport does");
        // On the racks, at the same height above the floor, facing −X, where the racks' pad faces.
        let rig = travel.rig();
        let head = rig.matrix().transform_point3(HEAD);
        assert!(close(head, Vec3::new(5.0, HEAD.y - 4.5, 1.0)), "{head}");
        assert_eq!(rig.floor(), -4.5);
        assert!(close(
            rig.matrix().transform_vector3(Vec3::NEG_Z),
            Vec3::NEG_X
        ));
        // Standing there, and swaying within 15 cm of its edge, it does not take them back.
        let racks = |offset: Vec3| short(rig.matrix().transform_point3(HEAD + offset));
        for offset in [Vec3::ZERO, Vec3::Z * 0.6, Vec3::ZERO] {
            for _ in 0..4 {
                assert!(!travel.update(racks(offset)).transported, "{offset}");
            }
        }
        // Once stepped off, back on, it does, facing as they faced, since the main floor's pad says nothing.
        travel.update(racks(Vec3::Z * 0.7));
        for _ in 0..4 {
            assert!(!travel.update(racks(Vec3::ZERO)).transported);
        }
        let facing = travel.rig().matrix().transform_vector3(Vec3::NEG_Z);
        assert!(travel.update(racks(Vec3::ZERO)).transported);
        let head = travel.rig().matrix().transform_point3(HEAD);
        assert!(close(head, Vec3::new(0.3, HEAD.y, -0.2)), "{head}");
        assert!(close(
            travel.rig().matrix().transform_vector3(Vec3::NEG_Z),
            facing
        ));
    }

    #[test]
    fn a_pad_chosen_with_the_ray_takes_the_participant_from_anywhere_while_travel_is_active() {
        let storeys = storeys();
        let racks = crate::pads::Key { pair: 0, end: 1 };
        let mut travel = Travel::default();
        let away = HEAD + Vec3::new(-3.0, 0.0, 2.0);
        let chose = |chosen, active: bool, head: Option<Vec3>| Input {
            chosen,
            active,
            head,
            ..input(&storeys, Vec2::ZERO, None)
        };
        // Inactive, as while the hand menu shows, without a head, or a pad not among the profile's, it goes nowhere.
        for nowhere in [
            chose(Some(racks), false, Some(away)),
            chose(Some(racks), true, None),
            chose(Some(crate::pads::Key { pair: 3, end: 0 }), true, Some(away)),
        ] {
            assert!(!travel.update(nowhere).transported);
        }
        assert_eq!(travel.rig(), Rig::default());
        // Pointed at the racks' pad and chosen, it takes them to the main floor's, where they stand: back where they
        // stand on the floor, a step from where they were, without stepping onto it.
        let outcome = travel.update(chose(Some(racks), true, Some(away)));
        assert!(outcome.transported);
        let head = travel.rig().matrix().transform_point3(away);
        assert!(close(head, Vec3::new(0.3, HEAD.y, -0.2)), "{head}");
        // Arrived on it, it is disarmed: standing there takes them nowhere.
        for _ in 0..4 {
            assert!(!travel.update(chose(None, true, Some(head))).transported);
        }
        // A push aimed before it lands nothing after it, until the stick returns to the middle.
        let aim = Some(ray(head, Vec3::new(2.0, 0.0, -3.0)));
        travel.update(input(&storeys, Vec2::Y, aim));
        travel.update(Input {
            chosen: Some(crate::pads::Key { pair: 0, end: 0 }),
            ..input(&storeys, Vec2::Y, aim)
        });
        assert!(!travel.update(input(&storeys, Vec2::ZERO, aim)).teleported);
    }

    #[test]
    fn a_pad_stood_on_while_travel_was_inactive_or_before_joining_takes_nobody_until_stepped_off_and_on()
     {
        let storeys = storeys();
        let beside = HEAD + Vec3::X * 2.0;
        let at = |head: Vec3, active: bool| Input {
            head: Some(head),
            active,
            dt: 0.125,
            ..input(&storeys, Vec2::ZERO, None)
        };
        let mut travel = Travel::default();
        travel.update(at(beside, true));
        // Stood on for part of the dwell, the hand menu opens: however long it shows, and after, the pad takes nobody.
        for _ in 0..3 {
            travel.update(at(HEAD, true));
        }
        for _ in 0..8 {
            assert!(!travel.update(at(HEAD, false)).transported);
        }
        for _ in 0..8 {
            assert!(!travel.update(at(HEAD, true)).transported);
        }
        // Stepped off and on, it does.
        travel.update(at(beside, true));
        let taken = (0..6).filter(|_| travel.update(at(HEAD, true)).transported);
        assert_eq!(taken.count(), 1);
        // Leaving and joining again standing on it: the same.
        let racks = travel.rig().matrix().transform_point3(HEAD);
        travel.update(Input {
            place: None,
            ..at(racks, true)
        });
        for _ in 0..8 {
            assert!(!travel.update(at(HEAD, true)).transported);
        }
    }

    #[test]
    fn a_pad_s_name_is_kept_short_and_plain() {
        let name = |name: &str| Pad::at(Vec3::ZERO).named(name).name().map(str::to_owned);
        assert_eq!(name("Racks"), Some("Racks".into()));
        assert_eq!(name("Ra\ncks\u{7}\t"), Some("Racks".into()));
        assert_eq!(name(&"x".repeat(100)), Some("x".repeat(LONGEST_NAME)));
        assert_eq!(name(&"あ".repeat(100)), Some("あ".repeat(LONGEST_NAME)));
        assert_eq!(name(""), None);
        assert_eq!(name("\n\u{0}"), None);
        // Carried, a name is shared rather than copied.
        let pad = Pad::at(Vec3::ZERO).named("Racks");
        let street = tomori_protocol::Pose::at([8.0, 0.5, 0.0]).facing_towards([0.0, 0.5, 0.0]);
        let carried = pad.carried(&crate::anchors::Anchor::carriage(&street).unwrap());
        assert!(std::ptr::eq(
            pad.name().unwrap().as_ptr(),
            carried.name().unwrap().as_ptr()
        ));
    }
}
