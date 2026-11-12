// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Travel beyond the room: the locomotion profile an environment declares, its walkable surfaces and where a ray lands on them, the participant's [`Rig`], and [`Travel`], which turns the left stick and the ray into teleports, snap turns, backsteps and rising. Pure, so that it is tested without a device; the frame step applies the rig to every pose. See `design/locomotion.md`.
use glam::{Mat4, Quat, Vec2, Vec3};

use crate::{anchors::Anchor, environment::Environment, pointing::Ray};

mod outline;

pub use outline::{CLEARANCE, FARTHEST, MOST_POINTS, Outline, OutlineError, SMALLEST_AREA};
pub use tomori_environment::navmesh::{Navmesh, Obstacle};

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
/// The most teleport targets a profile uses: those declared after them are ignored.
pub const MOST_TARGETS: usize = 8;
/// The tallest a teleport target may be, in metres: a taller one is ignored.
pub const TALLEST_TARGET: f32 = 4.0;
/// The most steps [`Surface::sector_triangles`] cuts a sector's sweep into.
pub const MOST_SEGMENTS: usize = 1024;

/// A surface a participant may land on, in the space's scene coordinates: horizontal, made with [`Surface::rectangle`], [`Surface::disc`], [`Surface::sector`] and [`Surface::polygon`], or the ground an environment draws, as triangles at their own heights, made with [`Surface::mesh`]; so that it can grow, as with colliders, without breaking:
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
#[derive(Clone, Debug, PartialEq)]
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
    /// The part of a ring about `centre`, from `inner` to `outer` of it, from the direction `from` round by `sweep`,
    /// anticlockwise seen from above, each an angle about the vertical from −Z, as a [`Pad`] faces. Made with
    /// [`Surface::sector`].
    #[non_exhaustive]
    Sector {
        centre: Vec3,
        inner: f32,
        outer: f32,
        from: f32,
        sweep: f32,
    },
    /// A polygon, its `outline` as it was declared, X and Z, standing with the outline's origin at `origin`, at its
    /// height, and turned `yaw` about the vertical from the scene's axes, as it is carried into a space (see
    /// [`Locomotion::carried`]). Made with [`Surface::polygon`].
    #[non_exhaustive]
    Polygon {
        outline: std::sync::Arc<Outline>,
        origin: Vec3,
        yaw: f32,
    },
    /// Ground as triangles, each at the heights it is drawn, such as [`Navmesh::derive`] derives from what an
    /// environment draws as ground less what it declares solid, in the coordinates it was declared in, standing with
    /// their origin at `origin` and turned `yaw` about the vertical, as it is carried into a space. Made with
    /// [`Surface::mesh`].
    #[non_exhaustive]
    Mesh {
        mesh: std::sync::Arc<Navmesh>,
        origin: Vec3,
        yaw: f32,
    },
}

/// Where `point`, seen from above, is in the frame of a surface standing at `origin` and turned `yaw` about the
/// vertical: X and Z from its origin, along its own axes.
fn within(point: Vec3, origin: Vec3, yaw: f32) -> Vec2 {
    let offset = Vec3::new(point.x - origin.x, 0.0, point.z - origin.z);
    let local = if yaw == 0.0 {
        offset
    } else {
        Quat::from_rotation_y(-yaw) * offset
    };
    Vec2::new(local.x, local.z)
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

    /// A ring sector, such as a curved deck: the part of the ring about `centre`, at its height, between the radii
    /// `inner` and `outer`, from the direction `from` round to `to`, anticlockwise seen from above. Exact, as a disc
    /// is. The angles are in radians about the vertical from −Z, as a [`Pad`] faces and the rig turns: 0 is ahead of
    /// the origin, along −Z, and a quarter turn, π/2, is along −X. A `to` less than `from` is a turn further round, and
    /// a full turn or more is the whole ring, a surface with a round hole. A `to` that comes back round to `from`, as
    /// `from` itself or a whole turn less, sweeps nothing and holds nothing, not even the line along `from`. The radii
    /// are taken without their signs and in order; an `inner` of nothing makes a wedge of a disc. To draw it, see
    /// [`Surface::sector_triangles`].
    ///
    /// ```
    /// use std::f32::consts::FRAC_PI_3;
    ///
    /// use glam::Vec3;
    /// use tomori_scene::locomotion::Surface;
    ///
    /// // A balcony 5.4 m up, 2 m deep, curving a third of the way round, 6 m ahead of the origin at its middle.
    /// let balcony = Surface::sector(Vec3::new(0.0, 5.4, 0.0), 6.0, 8.0, -FRAC_PI_3, FRAC_PI_3);
    /// assert!(balcony.contains(Vec3::new(0.0, 5.4, -7.0)));
    /// assert!(balcony.contains(Vec3::new(-5.0, 5.4, -5.0)));
    /// assert!(!balcony.contains(Vec3::new(0.0, 5.4, -5.0)));
    /// assert!(!balcony.contains(Vec3::new(0.0, 5.4, 7.0)));
    /// ```
    pub fn sector(centre: Vec3, inner: f32, outer: f32, from: f32, to: f32) -> Self {
        // A radius that is not a number stays one, which holds nothing.
        let (inner, outer) = match (inner.abs(), outer.abs()) {
            (inner, outer) if outer < inner => (outer, inner),
            radii => radii,
        };
        let sweep = to - from;
        Self::Sector {
            centre,
            inner,
            outer,
            from,
            // Nonsense stays nonsense, which holds nothing.
            sweep: if sweep.is_finite() && sweep >= std::f32::consts::TAU {
                std::f32::consts::TAU
            } else {
                sweep.rem_euclid(std::f32::consts::TAU)
            },
        }
    }

    /// A polygon at `height`: the outline through `points`, its corners in order, either way round, each the X and Z
    /// of a place in the environment's horizontal plane. Any simple outline, convex or not, without holes: from 3 to
    /// [`MOST_POINTS`] corners, each finite and within [`FARTHEST`] of the environment's origin on each axis, whose
    /// edges do not cross, which encloses at least [`SMALLEST_AREA`], and none of whose corners is within
    /// [`CLEARANCE`] of an edge it does not end; corners in a straight line are allowed. It is checked, and cut into
    /// triangles, once, here: an outline that is not one is refused, saying why, rather than making a surface nobody
    /// can land on. A point on the outline is on the surface, so four corners hold exactly what
    /// [`Surface::rectangle`] holds as declared; carried into a space, where a polygon turns about its outline's
    /// origin and a rectangle about its centre, they hold the same up to rounding, which near 1 km from the origin
    /// may move an edge by a tenth of a millimetre.
    ///
    /// ```
    /// use std::f32::consts::FRAC_PI_2;
    ///
    /// use glam::{Vec2, Vec3};
    /// use tomori_scene::locomotion::{Locomotion, OutlineError, Surface};
    ///
    /// // A crescent balcony 5.4 m up, 4 m deep at its middle, ahead of the origin: its outer edge half a circle of 8 m
    /// // about the origin, its inner edge an arc of 10 m about a point 6 m behind it, meeting at its horns.
    /// let arc = |centre: Vec2, radius: f32, from: f32, to: f32, steps: usize| {
    ///     (0..=steps).map(move |step| {
    ///         let angle = from + (to - from) * step as f32 / steps as f32;
    ///         centre + radius * Vec2::new(-angle.sin(), -angle.cos())
    ///     })
    /// };
    /// let horn = (4.0_f32 / 3.0).atan();
    /// let outer = arc(Vec2::ZERO, 8.0, FRAC_PI_2, -FRAC_PI_2, 24);
    /// let inner = arc(Vec2::new(0.0, 6.0), 10.0, -horn, horn, 16);
    /// // The arcs meet at the horns, which the outer one gives.
    /// let points = outer.chain(inner.skip(1).take(15));
    /// let crescent = Surface::polygon(points, 5.4)?;
    /// assert!(crescent.contains(Vec3::new(0.0, 5.4, -6.0)));
    /// assert!(!crescent.contains(Vec3::new(0.0, 5.4, -3.0)));
    /// let profile = Locomotion::fly(vec![crescent], 8.0);
    /// assert!(profile.travels());
    ///
    /// // Edges that cross are refused.
    /// let bow = [Vec2::ZERO, Vec2::new(2.0, 2.0), Vec2::new(2.0, 0.0), Vec2::new(0.0, 3.0)];
    /// assert!(matches!(Surface::polygon(bow, 0.0), Err(OutlineError::Crossing { .. })));
    /// # Ok::<(), OutlineError>(())
    /// ```
    pub fn polygon(
        points: impl IntoIterator<Item = Vec2>,
        height: f32,
    ) -> Result<Self, OutlineError> {
        Ok(Self::Polygon {
            outline: std::sync::Arc::new(Outline::new(points)?),
            origin: Vec3::new(0.0, height, 0.0),
            yaw: 0.0,
        })
    }

    /// The ground `mesh` holds, at the heights its triangles are, as walkable as [`Navmesh::derive`] made it: where an
    /// environment marks what it draws as ground and declares what is solid, so that where a participant lands is what
    /// is drawn there and the two never disagree. A point within one of its triangles, seen from above, is on it,
    /// where the highest triangle there is; a ray lands on the first it meets from above. Shared, not copied, as the
    /// profile is carried into each space.
    ///
    /// ```
    /// use glam::{Vec2, Vec3};
    /// use tomori_scene::locomotion::{Locomotion, Navmesh, Obstacle, Surface};
    /// use tomori_scene::pointing::Ray;
    ///
    /// // A yard 10 m square, drawn as two triangles, with a shed in it.
    /// let at = |x: f32, z: f32| Vec3::new(x, 0.0, z);
    /// let yard = [
    ///     [at(-5.0, -5.0), at(5.0, -5.0), at(5.0, 5.0)],
    ///     [at(-5.0, -5.0), at(5.0, 5.0), at(-5.0, 5.0)],
    /// ];
    /// let shed = Obstacle::cuboid(Vec3::new(1.0, 0.0, -4.0), Vec3::new(4.0, 2.4, -2.0));
    /// let ground = Surface::mesh(Navmesh::derive(yard, &[shed]));
    /// assert!(ground.contains(Vec3::new(0.0, 0.0, 0.0)));
    /// assert!(!ground.contains(Vec3::new(2.0, 0.0, -3.0)));
    /// let profile = Locomotion::teleport(vec![ground]);
    /// let ray = Ray::new(Vec3::new(0.0, 1.6, 0.0), Vec3::new(-1.0, -1.0, 0.0)).unwrap();
    /// assert!(profile.landing(&ray).unwrap().distance(Vec3::new(-1.6, 0.0, 0.0)) < 1e-5);
    /// ```
    pub fn mesh(mesh: Navmesh) -> Self {
        Self::Mesh {
            mesh: std::sync::Arc::new(mesh),
            origin: Vec3::ZERO,
            yaw: 0.0,
        }
    }

    /// The surface, given in an environment's own coordinates, in a space's, where `carriage` takes them: moved and
    /// turned with it.
    pub fn carried(&self, carriage: &crate::anchors::Anchor) -> Self {
        match self {
            &Self::Rectangle { centre, half, yaw } => Self::Rectangle {
                centre: carriage.place(centre),
                half,
                yaw: yaw + carriage.yaw(),
            },
            &Self::Disc { centre, radius } => Self::Disc {
                centre: carriage.place(centre),
                radius,
            },
            &Self::Sector {
                centre,
                inner,
                outer,
                from,
                sweep,
            } => Self::Sector {
                centre: carriage.place(centre),
                inner,
                outer,
                from: from + carriage.yaw(),
                sweep,
            },
            Self::Polygon {
                outline,
                origin,
                yaw,
            } => Self::Polygon {
                outline: outline.clone(),
                origin: carriage.place(*origin),
                yaw: yaw + carriage.yaw(),
            },
            Self::Mesh { mesh, origin, yaw } => Self::Mesh {
                mesh: mesh.clone(),
                origin: carriage.place(*origin),
                yaw: yaw + carriage.yaw(),
            },
        }
    }

    /// The surface's height; for a mesh, the lowest of its triangles' corners, or its origin's where it has none.
    pub fn height(&self) -> f32 {
        match self {
            Self::Rectangle { centre, .. }
            | Self::Disc { centre, .. }
            | Self::Sector { centre, .. } => centre.y,
            Self::Polygon { origin, .. } => origin.y,
            Self::Mesh { mesh, origin, .. } => {
                origin.y + mesh.bounds().map_or(0.0, |(low, _)| low.y)
            }
        }
    }

    /// The height of the floor it gives at `point`, seen from above: the highest of a mesh's triangles there, and
    /// another surface's own height, or `None` where `point` is not on it.
    pub fn height_at(&self, point: Vec3) -> Option<f32> {
        match self {
            Self::Mesh { mesh, origin, yaw } => mesh
                .height_at(within(point, *origin, *yaw))
                .map(|height| origin.y + height),
            _ => self.contains(point).then(|| self.height()),
        }
    }

    /// Whether someone may stand at `point` on it: on it, seen from above, on a floor within [`STANDING`] of
    /// `point`'s height, any of a mesh's where its triangles overlap.
    fn stands(&self, point: Vec3) -> bool {
        match self {
            Self::Mesh { mesh, origin, yaw } => mesh
                .heights(within(point, *origin, *yaw))
                .any(|height| (origin.y + height - point.y).abs() <= STANDING),
            _ => (self.height() - point.y).abs() <= STANDING && self.contains(point),
        }
    }

    /// How far along `ray` it meets the surface, at least nothing and less than `before`: from above, or, where
    /// `either_way`, from either side. A ray that runs level meets no horizontal surface.
    fn meets(&self, ray: &Ray, before: f32, either_way: bool) -> Option<f32> {
        match self {
            Self::Mesh { mesh, origin, yaw } => {
                let turn = Quat::from_rotation_y(-*yaw);
                mesh.meets(
                    turn * (ray.origin - *origin),
                    turn * ray.direction,
                    before,
                    either_way,
                )
                .filter(|distance| *distance < before)
            }
            _ => {
                if ray.direction.y.abs() < f32::EPSILON || !(either_way || ray.direction.y < 0.0) {
                    return None;
                }
                let distance = (self.height() - ray.origin.y) / ray.direction.y;
                (distance >= 0.0 && distance < before && self.contains(ray.at(distance)))
                    .then_some(distance)
            }
        }
    }

    /// A mesh's ground, in the coordinates it was declared in, wherever the surface has been carried, for whoever
    /// draws it or paths across it; `None` for the other shapes.
    pub fn navmesh(&self) -> Option<&Navmesh> {
        match self {
            Self::Mesh { mesh, .. } => Some(mesh),
            _ => None,
        }
    }

    /// A polygon's outline, in the coordinates it was declared in, wherever the surface has been carried, with the
    /// triangles Tomori cut it into, for whoever draws it; `None` for the other shapes.
    pub fn outline(&self) -> Option<&Outline> {
        match self {
            Self::Polygon { outline, .. } => Some(outline),
            _ => None,
        }
    }

    /// Triangles to draw a sector's floor with, at its height, in the coordinates its centre is in: its sweep cut into
    /// `segments` equal steps, from 1 to [`MOST_SEGMENTS`], each two triangles, or one where it has no inner radius.
    /// Empty for the other shapes and for a sector that holds nothing. They are not what it holds, which is exact:
    /// their edges are chords of its arcs, so the floor drawn falls short of its outer arc, and reaches into its
    /// hollow past its inner arc, by at most the radius times 1 − cos(sweep ÷ 2 · `segments`), as for 32 steps over
    /// half a ring of 10 m about 1.2 cm, and for 64 about 3 mm.
    ///
    /// ```
    /// use std::f32::consts::FRAC_PI_3;
    ///
    /// use glam::Vec3;
    /// use tomori_scene::locomotion::Surface;
    ///
    /// let balcony = Surface::sector(Vec3::new(0.0, 5.4, 0.0), 6.0, 8.0, -FRAC_PI_3, FRAC_PI_3);
    /// let floor = balcony.sector_triangles(32);
    /// assert_eq!(floor.len(), 64);
    /// assert!(floor.iter().flatten().all(|corner| corner.y == 5.4));
    /// ```
    pub fn sector_triangles(&self, segments: usize) -> Vec<[Vec3; 3]> {
        let &Self::Sector {
            centre,
            inner,
            outer,
            from,
            sweep,
        } = self
        else {
            return Vec::new();
        };
        // Nonsense, or a sweep of nothing, holds nothing; a start that is not a number has a sweep that is not either.
        if !(sweep > 0.0 && inner.is_finite() && outer.is_finite()) {
            return Vec::new();
        }
        let segments = segments.clamp(1, MOST_SEGMENTS);
        let at = |step: usize, radius: f32| {
            let angle = from + sweep * step as f32 / segments as f32;
            centre + radius * Vec3::new(-angle.sin(), 0.0, -angle.cos())
        };
        let mut triangles = Vec::with_capacity(2 * segments);
        for step in 0..segments {
            let (near, far) = (at(step, outer), at(step + 1, outer));
            if inner == 0.0 {
                triangles.push([centre, near, far]);
            } else {
                let (back, ahead) = (at(step, inner), at(step + 1, inner));
                triangles.push([back, near, far]);
                triangles.push([back, far, ahead]);
            }
        }
        triangles
    }

    /// Whether `point`, seen from above, is on the surface, its edge included.
    pub fn contains(&self, point: Vec3) -> bool {
        match self {
            &Self::Rectangle { centre, half, yaw } => {
                let local = within(point, centre, yaw);
                local.x.abs() <= half.x && local.y.abs() <= half.y
            }
            &Self::Disc { centre, radius } => {
                Vec2::new(point.x - centre.x, point.z - centre.z).length() <= radius
            }
            &Self::Sector {
                centre,
                inner,
                outer,
                from,
                sweep,
            } => {
                let offset = Vec2::new(point.x - centre.x, point.z - centre.z);
                let radius = offset.length();
                // The centre faces no way: it is on a wedge that reaches it.
                let round = if radius == 0.0 {
                    0.0
                } else {
                    ((-offset.x).atan2(-offset.y) - from).rem_euclid(std::f32::consts::TAU)
                };
                // A sweep of nothing holds nothing, rather than the line along its start.
                sweep > 0.0 && (inner..=outer).contains(&radius) && round <= sweep
            }
            Self::Polygon {
                outline,
                origin,
                yaw,
            } => outline.contains(within(point, *origin, *yaw)),
            Self::Mesh { mesh, origin, yaw } => {
                mesh.heights(within(point, *origin, *yaw)).next().is_some()
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
            && surfaces.iter().any(|surface| surface.stands(self.centre))
    }
}

/// A teleport target: an object of the environment's own, such as a rock to stand on, that takes a participant who
/// points at it and chooses it to where it says, as a pad's partner does, and never by stepping onto it. The ray points
/// at it where it meets the upright cylinder about its foot, as wide and as tall as it says, which the environment
/// draws; Tomori draws only, while it is pointed at, the ring where the participant will stand. A participant arrives on
/// its top's middle unless it says elsewhere, and may say which way they face and its name, which the label beside it
/// shows. Its top is a floor for whoever it takes there, not a surface a teleport lands on, so that it is reached only by
/// pointing at it; two targets, one at each end, make a way there and back. See `design/locomotion.md`.
///
/// ```
/// use glam::{Vec2, Vec3};
/// use tomori_scene::locomotion::{Locomotion, Surface, Target};
///
/// let shore = Surface::rectangle(Vec3::ZERO, Vec2::new(10.0, 10.0));
/// let rock = Target::object(Vec3::new(0.0, -0.8, -9.0), 0.9, 1.1)
///     .named("Rock")
///     .facing(Vec3::NEG_Z);
/// let back = Target::object(Vec3::new(1.0, 0.0, -1.0), 0.3, 1.0).arriving(Vec3::new(0.0, 0.0, -1.0));
/// let profile = Locomotion::teleport(vec![shore]).with_targets([rock, back]);
/// assert_eq!(profile.targets().count(), 2);
/// assert_eq!(profile.targets().next().unwrap().arrival(), Vec3::new(0.0, 0.3, -9.0));
/// ```
#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    foot: Vec3,
    radius: f32,
    height: f32,
    arrival: Vec3,
    /// Which way a participant arriving faces, as a turn about the vertical from −Z, as [`Rig`] turns, or `None` for the
    /// way they faced.
    yaw: Option<f32>,
    name: Option<std::sync::Arc<str>>,
    /// How near the middle of its foot the ray's origin must be for it to be pointed at, or `None` for anywhere within
    /// [`REACH`].
    within: Option<f32>,
}

impl Target {
    /// An object standing on `foot`, the middle of its base, pointed at within `radius` of its upright axis and up to
    /// `height` above its foot, on whose top's middle a participant arrives, facing the way they faced, unnamed, and
    /// pointed at from anywhere within [`REACH`].
    pub fn object(foot: Vec3, radius: f32, height: f32) -> Self {
        Self {
            foot,
            radius: radius.abs(),
            height,
            arrival: foot + Vec3::Y * height,
            yaw: None,
            name: None,
            within: None,
        }
    }

    /// The target, pointed at only from within `reach` of the middle of its foot, as a teleporter in a waterfall is
    /// reached only from the rock before it, rather than from wherever the fall is in sight.
    pub fn within(mut self, reach: f32) -> Self {
        self.within = Some(reach);
        self
    }

    /// How near the middle of its foot the ray must start for it to be pointed at, if it says.
    pub fn reach(&self) -> Option<f32> {
        self.within
    }

    /// Whether a ray from `origin` may point at it: from anywhere, or from within its reach of the middle of its foot.
    pub fn reached_from(&self, origin: Vec3) -> bool {
        self.within
            .is_none_or(|reach| origin.distance(self.foot) <= reach)
    }

    /// The target, where a participant arrives at `arrival` instead, the room's floor at its height: on a declared
    /// surface, or still on its top.
    pub fn arriving(mut self, arrival: Vec3) -> Self {
        self.arrival = arrival;
        self
    }

    /// The target, where a participant arriving faces `direction`, horizontally, as a pad's does.
    pub fn facing(mut self, direction: Vec3) -> Self {
        self.yaw = Pad::at(Vec3::ZERO).facing(direction).yaw;
        self
    }

    /// The target, called `name`, which the label beside it shows, kept as a pad's name is.
    pub fn named(mut self, name: impl AsRef<str>) -> Self {
        self.name = Pad::at(Vec3::ZERO).named(name).name;
        self
    }

    /// The middle of its base.
    pub fn foot(&self) -> Vec3 {
        self.foot
    }

    /// How far from its axis it is pointed at.
    pub fn radius(&self) -> f32 {
        self.radius
    }

    /// How tall it is from its foot.
    pub fn height(&self) -> f32 {
        self.height
    }

    /// Where a participant arrives, the room's floor at its height.
    pub fn arrival(&self) -> Vec3 {
        self.arrival
    }

    /// Which way a participant arriving faces, as a turn about the vertical from −Z, if it says.
    pub fn yaw(&self) -> Option<f32> {
        self.yaw
    }

    /// What it is called, if anything.
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// Whether `point`, seen from above, is within its radius of its axis.
    pub fn over(&self, point: Vec3) -> bool {
        Vec2::new(point.x - self.foot.x, point.z - self.foot.z).length() <= self.radius
    }

    /// Whether its arrival is on its own top: at its height, within a centimetre, and within its radius.
    pub fn arrives_on_top(&self) -> bool {
        self.holds(self.arrival)
    }

    /// Whether `point` stands on its top: at its height, within a centimetre, and within its radius.
    fn holds(&self, point: Vec3) -> bool {
        (point.y - (self.foot.y + self.height)).abs() <= STANDING && self.over(point)
    }

    /// The target, declared in an environment's own coordinates, in a space's, where `carriage` takes them: moved and
    /// turned with it, its axis still upright.
    pub fn carried(&self, carriage: &crate::anchors::Anchor) -> Self {
        Self {
            foot: carriage.place(self.foot),
            arrival: carriage.place(self.arrival),
            yaw: self.yaw.map(|yaw| yaw + carriage.yaw()),
            ..self.clone()
        }
    }

    /// Whether a participant may be taken to it: finite, no wider than [`WIDEST_PAD`] from its axis, no taller than
    /// [`TALLEST_TARGET`], it and its arrival within [`crate::anchors::REACH`] of home on each axis, reached from within
    /// a reach that is a positive number if it says one, and arriving on its own top, on one of `surfaces`, or on the
    /// top of another of `targets` that arrives on its own and may be used, so that whoever arrives stands on a floor:
    /// two targets, one on a rock and one on a terrace above it, so make a way up and back down.
    fn usable(&self, surfaces: &[Surface], targets: &[Target]) -> bool {
        let within =
            |point: Vec3| point.is_finite() && point.abs().max_element() <= crate::anchors::REACH;
        let onto = |other: &Target| {
            other.arrives_on_top() && other.holds(self.arrival) && other.usable(surfaces, &[])
        };
        within(self.foot)
            && within(self.foot + Vec3::Y * self.height)
            && within(self.arrival)
            && self.radius > 0.0
            && self.radius <= WIDEST_PAD
            && self.height > 0.0
            && self.height <= TALLEST_TARGET
            && self.yaw.is_none_or(f32::is_finite)
            && self.within.is_none_or(|reach| reach > 0.0)
            && (self.arrives_on_top()
                || surfaces.iter().any(|surface| surface.stands(self.arrival))
                || targets.iter().any(onto))
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
    /// Teleport onto the walkable `surfaces`, and turn in snaps, between each pair of `pads`, and to each of `targets`.
    /// Made with [`Locomotion::teleport`].
    #[non_exhaustive]
    Teleport {
        surfaces: Vec<Surface>,
        pads: Vec<[Pad; 2]>,
        targets: Vec<Target>,
    },
    /// Teleport and turn, and rise as well, never above `ceiling`, the highest the room's floor goes, between each pair
    /// of `pads`, and to each of `targets`. Made with [`Locomotion::fly`].
    #[non_exhaustive]
    Fly {
        surfaces: Vec<Surface>,
        ceiling: f32,
        pads: Vec<[Pad; 2]>,
        targets: Vec<Target>,
    },
}

impl Locomotion {
    /// Teleport onto `surfaces`, and turn in snaps.
    pub fn teleport(surfaces: Vec<Surface>) -> Self {
        Self::Teleport {
            surfaces,
            pads: Vec::new(),
            targets: Vec::new(),
        }
    }

    /// Teleport onto `surfaces`, turn in snaps, and rise up to `ceiling`.
    pub fn fly(surfaces: Vec<Surface>, ceiling: f32) -> Self {
        Self::Fly {
            surfaces,
            ceiling,
            pads: Vec::new(),
            targets: Vec::new(),
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
            Self::Teleport { surfaces, pads, .. } | Self::Fly { surfaces, pads, .. } => {
                (surfaces, pads)
            }
        };
        pads.iter()
            .filter(move |pair| pair.iter().all(|pad| pad.usable(surfaces)))
            .take(MOST_PAIRS)
    }

    /// The profile, with `targets` as well, each taking a participant who points at it and chooses it there. A profile
    /// of none has no targets, since nothing travels there. Only the first [`MOST_TARGETS`] of those that may be used
    /// are, as [`Self::targets`] gives them.
    pub fn with_targets(mut self, more: impl IntoIterator<Item = Target>) -> Self {
        if let Self::Teleport { targets, .. } | Self::Fly { targets, .. } = &mut self {
            targets.extend(more);
        }
        self
    }

    /// The teleport targets a participant may use: the first [`MOST_TARGETS`] of those declared that are finite, no
    /// wider than [`WIDEST_PAD`], no taller than [`TALLEST_TARGET`], within [`crate::anchors::REACH`] of home on each
    /// axis and arriving on their own top, on a declared surface or on another's top.
    pub fn targets(&self) -> impl Iterator<Item = &Target> {
        let (surfaces, targets): (&[Surface], &[Target]) = match self {
            Self::None => (&[], &[]),
            Self::Teleport {
                surfaces, targets, ..
            }
            | Self::Fly {
                surfaces, targets, ..
            } => (surfaces, targets),
        };
        targets
            .iter()
            .filter(move |target| target.usable(surfaces, targets))
            .take(MOST_TARGETS)
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
        let aims = |targets: &[Target]| -> Vec<Target> {
            targets
                .iter()
                .map(|target| target.carried(carriage))
                .collect()
        };
        match self {
            Self::None => Self::None,
            Self::Teleport {
                surfaces,
                pads,
                targets,
            } => Self::teleport(carry(surfaces))
                .with_pads(pairs(pads))
                .with_targets(aims(targets)),
            Self::Fly {
                surfaces,
                ceiling,
                pads,
                targets,
            } => Self::fly(carry(surfaces), ceiling + carriage.position.y)
                .with_pads(pairs(pads))
                .with_targets(aims(targets)),
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
        self.surfaces()
            .iter()
            .any(|surface| surface.meets(ray, distance, true).is_some())
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
                let distance = surface.meets(ray, f32::INFINITY, false)?;
                let point = ray.at(distance);
                // At the height of the floor it meets there, as declared or drawn, rather than as the ray rounds it:
                // of a mesh's, the one nearest where the ray is.
                let height = match surface {
                    Surface::Mesh { mesh, origin, yaw } => mesh
                        .heights(within(point, *origin, *yaw))
                        .map(|height| origin.y + height)
                        .min_by(|a, b| (a - point.y).abs().total_cmp(&(b - point.y).abs()))
                        .unwrap_or(point.y),
                    _ => surface.height(),
                };
                Some((distance, point.with_y(height)))
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
                    let point = (head + back * distance).with_y(floor);
                    surfaces.iter().any(|surface| surface.stands(point))
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
    /// The teleport pad or target the participant chose with the ray this frame, if any, as `crate::pads` keys it
    /// among the profile's.
    pub chosen: Option<crate::pads::Way>,
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
        // it, and a target chosen with the ray to itself, as the frame's poses were placed, before the stick moves
        // anything, and nothing else moves them this frame.
        let pairs: Vec<&[Pad; 2]> = input.locomotion.pads().collect();
        let stepped = self
            .stepping
            .update(&pairs, input.head, self.rig.floor(), dt)
            .map(crate::pads::Way::Pad);
        if let (Some(way), Some(head)) = (input.chosen.or(stepped), input.head)
            && let Some((arrival, yaw)) = crate::pads::destination(input.locomotion, way)
        {
            self.rig.teleport(arrival, head);
            if let (Some(yaw), Some(facing)) = (yaw, input.facing) {
                let faced = (-facing.x).atan2(-facing.z);
                self.rig.turn(yaw - faced, arrival.with_y(head.y));
            }
            // The pad arrived on is disarmed until stepped off; a target's arrival disarms whatever pad it is on.
            match way {
                crate::pads::Way::Pad(key) => self.stepping.arrive(key.partner()),
                crate::pads::Way::Target(_) => self.stepping.pause(),
            }
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
mod tests;
