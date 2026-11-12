// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! What an environment declares: its loose bodies, each a convex hull with what it is made of, physical or only
//! grabbable; the static colliders they rest on, in the environment's own coordinates; its water; and the world's
//! settings.
use glam::{Quat, Vec2, Vec3};

use crate::{HULL_POINTS, Placement, Refused};

/// Whether a body is simulated once let go, as a stone or a stick that falls, rolls and rests on whatever is beneath
/// it, or only grabbable, carried in the hand and put back where its environment's side decides, as something that
/// hangs on a hook would be.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Kind {
    #[default]
    Dynamic,
    Kinematic,
}

/// One of an environment's loose bodies, by the number it is known by on the wire: the points of its convex hull, which
/// it is drawn and collided from alike, in its own coordinates, about its middle; how dense it is, its friction, how
/// much it bounces and how its motion is damped; whether it is [`Kind::Dynamic`] or [`Kind::Kinematic`]; and where it
/// begins, which it returns to when it is lost.
#[derive(Clone, Debug, PartialEq)]
pub struct Body {
    pub id: u32,
    pub hull: Vec<Vec3>,
    pub density: f32,
    pub friction: f32,
    pub restitution: f32,
    pub linear_damping: f32,
    pub angular_damping: f32,
    pub kind: Kind,
    pub home: Placement,
}

impl Body {
    /// Body `id`, the convex hull of `points`, of granite by default: 2,650 kg/m³, a friction of 0.9, bouncing little,
    /// damped a little, dynamic, beginning at the origin. A face that another body is to rest on, or that rests on
    /// another, has at most four corners: Rapier rests a face of more than four on another unsteadily, so that a stack
    /// of them shakes itself apart. See `design/physics.md`.
    pub fn convex(id: u32, points: Vec<Vec3>) -> Self {
        Self {
            id,
            hull: points,
            density: 2_650.0,
            friction: 0.9,
            restitution: 0.05,
            linear_damping: 0.1,
            angular_damping: 0.5,
            kind: Kind::Dynamic,
            home: Placement::IDENTITY,
        }
    }

    pub fn with_density(mut self, density: f32) -> Self {
        self.density = density;
        self
    }

    pub fn with_friction(mut self, friction: f32) -> Self {
        self.friction = friction;
        self
    }

    pub fn with_restitution(mut self, restitution: f32) -> Self {
        self.restitution = restitution;
        self
    }

    pub fn with_damping(mut self, linear: f32, angular: f32) -> Self {
        self.linear_damping = linear;
        self.angular_damping = angular;
        self
    }

    /// This body, only grabbable: let go, it does not fall, but stays where its side puts it.
    pub fn kinematic(mut self) -> Self {
        self.kind = Kind::Kinematic;
        self
    }

    /// This body, beginning at `home`.
    pub fn at(mut self, home: Placement) -> Self {
        self.home = home;
        self
    }

    /// How far its farthest point is from its middle, the radius of the sphere it turns within.
    pub fn radius(&self) -> f32 {
        self.hull
            .iter()
            .map(|point| point.length())
            .fold(0.0, f32::max)
    }

    /// How far below its middle its lowest point is, placed by `placement`.
    pub fn lowest(&self, placement: &Placement) -> f32 {
        self.hull
            .iter()
            .map(|point| placement.transform_point(*point).y)
            .fold(f32::INFINITY, f32::min)
    }

    /// Whether the world takes it, or why not: a hull of 4 to [`HULL_POINTS`] finite points, positive and finite
    /// density and friction, restitution and damping finite and not negative, and a valid home.
    pub fn check(&self) -> Result<(), Refused> {
        if self.hull.len() < 4 || self.hull.len() > HULL_POINTS {
            return Err(Refused::Hull(self.id));
        }
        let finite = self.hull.iter().all(|point| point.is_finite());
        let made = [
            self.density,
            self.friction,
            self.restitution + 1.0,
            self.linear_damping + 1.0,
            self.angular_damping + 1.0,
        ]
        .iter()
        .all(|value| value.is_finite() && *value > 0.0);
        if !finite || !made || !self.home.valid() {
            return Err(Refused::Body(self.id));
        }
        Ok(())
    }
}

/// Something bodies rest on and bounce off that never moves, in the environment's own coordinates.
#[derive(Clone, Debug, PartialEq)]
pub enum Static {
    /// Triangles, by the corners in `vertices` each names: what the scenery draws, so that a body rests on what is seen.
    Mesh {
        vertices: Vec<Vec3>,
        triangles: Vec<[u32; 3]>,
    },
    /// The convex hull of `points`, such as a boulder.
    Convex { points: Vec<Vec3> },
    /// A box, its middle where `placement` puts it, `half` its half-extents.
    Cuboid { placement: Placement, half: Vec3 },
}

impl Static {
    /// A grid of heights, `columns` across along +X and as many rows along +Z as `heights` holds, `spacing` apart,
    /// beginning at `corner` on the ground, as triangles, two to each cell.
    pub fn heights(corner: Vec2, spacing: f32, columns: usize, heights: &[f32]) -> Self {
        let columns = columns.max(1);
        let rows = heights.len() / columns;
        let vertices = (0..rows * columns)
            .map(|index| {
                let (row, column) = (index / columns, index % columns);
                Vec3::new(
                    corner.x + column as f32 * spacing,
                    heights[index],
                    corner.y + row as f32 * spacing,
                )
            })
            .collect();
        let mut triangles = Vec::new();
        for row in 0..rows.saturating_sub(1) {
            for column in 0..columns - 1 {
                let at = |row: usize, column: usize| (row * columns + column) as u32;
                let (a, b) = (at(row, column), at(row, column + 1));
                let (c, d) = (at(row + 1, column + 1), at(row + 1, column));
                triangles.push([a, d, c]);
                triangles.push([a, c, b]);
            }
        }
        Self::Mesh {
            vertices,
            triangles,
        }
    }

    /// Triangles, as [`Static::Mesh`].
    pub fn mesh(vertices: Vec<Vec3>, triangles: Vec<[u32; 3]>) -> Self {
        Self::Mesh {
            vertices,
            triangles,
        }
    }

    pub fn convex(points: Vec<Vec3>) -> Self {
        Self::Convex { points }
    }

    /// A box from `least` to `greatest`, turned by `rotation` about its middle.
    pub fn cuboid(least: Vec3, greatest: Vec3, rotation: Quat) -> Self {
        Self::Cuboid {
            placement: Placement::new((least + greatest) * 0.5, rotation),
            half: (greatest - least).abs() * 0.5,
        }
    }

    /// How many triangles it counts as against [`crate::STATIC_TRIANGLES`]: a mesh's own, and a hull's or a box's
    /// twelve, as a box has.
    pub fn triangles(&self) -> usize {
        match self {
            Self::Mesh { triangles, .. } => triangles.len(),
            Self::Convex { .. } | Self::Cuboid { .. } => 12,
        }
    }

    /// Its lowest point.
    pub fn lowest(&self) -> f32 {
        match self {
            Self::Mesh { vertices, .. } => vertices
                .iter()
                .map(|point| point.y)
                .fold(f32::INFINITY, f32::min),
            Self::Convex { points } => points
                .iter()
                .map(|point| point.y)
                .fold(f32::INFINITY, f32::min),
            Self::Cuboid { placement, half } => {
                let mut lowest = f32::INFINITY;
                for corner in 0..8 {
                    let sign = |bit: u32| if corner & bit == 0 { -1.0 } else { 1.0 };
                    let local = *half * Vec3::new(sign(1), sign(2), sign(4));
                    lowest = lowest.min(placement.transform_point(local).y);
                }
                lowest
            }
        }
    }

    /// Whether it is one the world takes: finite, a mesh's triangles naming its corners, a hull of 4 to
    /// [`HULL_POINTS`] points, a box with an extent every way.
    pub fn valid(&self) -> bool {
        match self {
            Self::Mesh {
                vertices,
                triangles,
            } => {
                !triangles.is_empty()
                    && vertices.iter().all(|point| point.is_finite())
                    && triangles
                        .iter()
                        .flatten()
                        .all(|corner| (*corner as usize) < vertices.len())
            }
            Self::Convex { points } => {
                (4..=HULL_POINTS).contains(&points.len())
                    && points.iter().all(|point| point.is_finite())
            }
            Self::Cuboid { placement, half } => {
                placement.valid() && half.is_finite() && half.min_element() > 0.0
            }
        }
    }
}

/// A body of water: everything below `height` within `outline` on the ground, in the environment's own coordinates,
/// which slows what enters it, with more damping, and buoys it, by the weight of the water it displaces; and, if it
/// says, takes a body that comes to rest in it back to where it began after a while, as a river does a stone thrown in.
#[derive(Clone, Debug, PartialEq)]
pub struct Water {
    pub height: f32,
    pub outline: Vec<Vec2>,
    pub density: f32,
    pub linear_damping: f32,
    pub angular_damping: f32,
    /// How long, in milliseconds, a body rests in it before it goes back where it began, if it ever does.
    pub returns_after: Option<u64>,
}

impl Water {
    /// Water up to `height` within `outline`, as dense as fresh water, damping linear motion by 6 and turning by 3, so
    /// that a stone sinks at about a metre a second, as a stone does.
    pub fn below(height: f32, outline: Vec<Vec2>) -> Self {
        Self {
            height,
            outline,
            density: 1_000.0,
            linear_damping: 6.0,
            angular_damping: 3.0,
            returns_after: None,
        }
    }

    /// This water, taking a body that rests in it for `after` milliseconds back to where it began.
    pub fn returning(mut self, after: u64) -> Self {
        self.returns_after = Some(after);
        self
    }

    /// Whether `point` is in it: below its height and within its outline.
    pub fn contains(&self, point: Vec3) -> bool {
        point.y < self.height && within(&self.outline, Vec2::new(point.x, point.z))
    }
}

/// Whether `point` is inside the polygon `outline`, by the crossings of a line from it.
pub fn within(outline: &[Vec2], point: Vec2) -> bool {
    let mut inside = false;
    for index in 0..outline.len() {
        let (a, b) = (outline[index], outline[(index + 1) % outline.len()]);
        if (a.y > point.y) != (b.y > point.y)
            && point.x < a.x + (point.y - a.y) / (b.y - a.y) * (b.x - a.x)
        {
            inside = !inside;
        }
    }
    inside
}

/// How the world simulates: its gravity; its step and how many substeps each takes; when a body falls asleep, once it
/// has moved less than `sleep_speed` a second and turned less than `sleep_turn` radians a second for `sleep_after`
/// seconds, firmer than Rapier's own, so that a cairn stops being simulated as soon as it stands; and how far from home
/// and below its lowest collider a body is lost, and goes back where it began.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    pub gravity: Vec3,
    pub step: f32,
    pub substeps: usize,
    pub sleep_speed: f32,
    pub sleep_turn: f32,
    pub sleep_after: f32,
    pub home: Vec3,
    pub lost_beyond: f32,
    pub lost_below: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            gravity: Vec3::new(0.0, -9.81, 0.0),
            step: 1.0 / 60.0,
            substeps: 4,
            sleep_speed: 0.005,
            sleep_turn: 0.03,
            sleep_after: 0.5,
            home: Vec3::ZERO,
            lost_beyond: 30.0,
            lost_below: 1.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tetrahedron() -> Vec<Vec3> {
        vec![
            Vec3::new(0.1, 0.0, 0.0),
            Vec3::new(-0.1, 0.0, 0.05),
            Vec3::new(0.0, 0.0, -0.1),
            Vec3::new(0.0, 0.05, 0.0),
        ]
    }

    #[test]
    fn a_body_is_granite_until_it_says_otherwise() {
        let body = Body::convex(3, tetrahedron());
        assert_eq!(
            (body.density, body.friction, body.restitution, body.kind),
            (2_650.0, 0.9, 0.05, Kind::Dynamic)
        );
        let body = body
            .with_density(800.0)
            .with_friction(0.5)
            .with_restitution(0.2)
            .with_damping(0.3, 0.4)
            .kinematic()
            .at(Placement::at(Vec3::Y));
        assert_eq!(
            (body.density, body.friction, body.restitution),
            (800.0, 0.5, 0.2)
        );
        assert_eq!((body.linear_damping, body.angular_damping), (0.3, 0.4));
        assert_eq!(body.kind, Kind::Kinematic);
        assert_eq!(body.home, Placement::at(Vec3::Y));
        assert!((body.radius() - 0.0125f32.sqrt()).abs() < 1e-6);
        assert!((body.lowest(&Placement::at(Vec3::Y)) - 1.0).abs() < 1e-6);
        assert_eq!(body.check(), Ok(()));
    }

    #[test]
    fn a_body_the_world_cannot_take_is_refused() {
        let few = Body::convex(1, tetrahedron()[..3].to_vec());
        assert_eq!(few.check(), Err(Refused::Hull(1)));
        let many = Body::convex(2, vec![Vec3::X; HULL_POINTS + 1]);
        assert_eq!(many.check(), Err(Refused::Hull(2)));
        let mut points = tetrahedron();
        points[0].x = f32::NAN;
        assert_eq!(Body::convex(3, points).check(), Err(Refused::Body(3)));
        for body in [
            Body::convex(4, tetrahedron()).with_density(0.0),
            Body::convex(4, tetrahedron()).with_friction(f32::INFINITY),
            Body::convex(4, tetrahedron()).with_restitution(-2.0),
            Body::convex(4, tetrahedron()).with_damping(-1.0, 0.0),
            Body::convex(4, tetrahedron()).with_damping(0.0, f32::NAN),
            Body::convex(4, tetrahedron()).at(Placement::at(Vec3::NAN)),
        ] {
            assert_eq!(body.check(), Err(Refused::Body(4)));
        }
    }

    #[test]
    fn heights_are_two_triangles_to_a_cell() {
        let grid = Static::heights(Vec2::new(1.0, 2.0), 0.5, 3, &[0.0, 0.1, 0.2, 0.3, 0.4, 0.5]);
        let Static::Mesh {
            vertices,
            triangles,
        } = &grid
        else {
            unreachable!()
        };
        assert_eq!(vertices.len(), 6);
        assert_eq!(vertices[4], Vec3::new(1.5, 0.4, 2.5));
        assert_eq!(triangles.len(), 4);
        assert_eq!(grid.triangles(), 4);
        assert_eq!(grid.lowest(), 0.0);
        assert!(grid.valid());
        // A single row has no cells, and is no mesh the world takes.
        let row = Static::heights(Vec2::ZERO, 1.0, 0, &[0.0, 1.0]);
        assert_eq!(row.triangles(), 0);
        assert!(!row.valid());
        assert!(!Static::mesh(vec![Vec3::ZERO], vec![[0, 0, 1]]).valid());
        assert!(!Static::mesh(vec![Vec3::NAN; 3], vec![[0, 1, 2]]).valid());
    }

    #[test]
    fn hulls_and_boxes_count_as_twelve_triangles_and_say_their_lowest_point() {
        let hull = Static::convex(tetrahedron());
        assert_eq!((hull.triangles(), hull.lowest()), (12, 0.0));
        assert!(hull.valid());
        assert!(!Static::convex(tetrahedron()[..3].to_vec()).valid());
        assert!(!Static::convex(vec![Vec3::NAN; 4]).valid());
        let tilted = Static::cuboid(
            Vec3::new(-1.0, 0.0, -1.0),
            Vec3::new(1.0, 1.0, 1.0),
            Quat::from_rotation_z(std::f32::consts::FRAC_PI_4),
        );
        assert_eq!(tilted.triangles(), 12);
        assert!((tilted.lowest() - (0.5 - 1.5 / 2f32.sqrt())).abs() < 1e-5);
        assert!(tilted.valid());
        assert!(!Static::cuboid(Vec3::ZERO, Vec3::new(1.0, 0.0, 1.0), Quat::IDENTITY).valid());
        assert!(!Static::cuboid(Vec3::ZERO, Vec3::NAN, Quat::IDENTITY).valid());
    }

    #[test]
    fn water_holds_what_is_below_it_within_its_outline() {
        let square = vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(2.0, 0.0),
            Vec2::new(2.0, 2.0),
            Vec2::new(0.0, 2.0),
        ];
        let water = Water::below(-0.5, square.clone());
        assert_eq!(water.returns_after, None);
        assert!(water.contains(Vec3::new(1.0, -0.6, 1.0)));
        assert!(!water.contains(Vec3::new(1.0, -0.4, 1.0)));
        assert!(!water.contains(Vec3::new(3.0, -0.6, 1.0)));
        assert_eq!(water.returning(60_000).returns_after, Some(60_000));
        assert!(within(&square, Vec2::new(0.5, 1.5)));
        assert!(!within(&square, Vec2::new(-0.5, 1.5)));
        assert!(!within(&[], Vec2::ZERO));
    }

    #[test]
    fn the_settings_sleep_firmly() {
        let settings = Settings::default();
        assert_eq!(settings.substeps, 4);
        assert_eq!(
            (
                settings.sleep_speed,
                settings.sleep_turn,
                settings.sleep_after
            ),
            (0.005, 0.03, 0.5)
        );
        assert_eq!((settings.lost_beyond, settings.lost_below), (30.0, 1.0));
    }
}
