// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Walkable ground derived from what an environment draws, rather than outlined by hand: the triangles it marks as
//! ground, kept where they slope no more than [`STEEPEST`], less what it declares solid, each [`Obstacle`] grown by
//! its clearance, indexed on a grid so that a point or a ray finds the triangles beneath it at once. Heights are the
//! drawn triangles' own, so that where a participant lands is on what is drawn there. Pure, with no device, for a
//! client's teleport profile (`tomori-scene`'s `Surface::mesh`) and, later, for a server side that paths across it.
//! See Tomori's `design/locomotion.md`.
use glam::{Vec2, Vec3};

/// The steepest ground a [`Navmesh`] keeps, in radians from the horizontal: 30°.
pub const STEEPEST: f32 = std::f32::consts::FRAC_PI_6;
/// How far from an [`Obstacle`]'s footprint ground is cut away unless it says otherwise, in metres.
pub const CLEARANCE: f32 = 0.25;
/// How tall the room above ground is that someone standing on it needs, in metres: an obstacle cuts ground only where
/// it reaches into the room above it, so that ground beneath a high bough or a bridge stays, and so does ground above
/// something buried.
pub const HEADROOM: f32 = 2.0;
/// The most triangles a [`Navmesh`] keeps: those beyond them are dropped, so that nonsense cannot exhaust memory.
pub const MOST_TRIANGLES: usize = 1 << 16;
/// The size of the grid's cells, in metres, unless the mesh is so wide that it would have more than [`MOST_CELLS`].
const CELL: f32 = 1.0;
const MOST_CELLS: f32 = 65536.0;
/// How far outside a triangle a point may be, in metres, and still be on it, so that no seam between the triangles of
/// one floor lets a point or a ray through.
const SEAM: f32 = 1e-5;
/// The least a piece of ground is kept at, in square metres: less, it is a sliver left by cutting.
const SLIVER: f32 = 1e-6;

/// Something an environment declares solid, which ground is cut away about: its footprint, the convex hull of the
/// corners it is given, seen from above, X and Z; how high it reaches, from its `bottom` to its `top`; and its
/// clearance, how far about its footprint the ground is cut, [`CLEARANCE`] unless it says otherwise. The cut is the
/// footprint grown by the clearance along every edge, mitred, but no further than its bounding box grown by the
/// clearance, so that a box grows into a bigger box, as a rectangle grown on every side does, and a sharp corner is
/// cut off rather than reaching far. It cuts only ground whose room, [`HEADROOM`] above it, it reaches into.
///
/// ```
/// use glam::{Vec2, Vec3};
/// use tomori_environment::navmesh::{Navmesh, Obstacle};
///
/// // A floor 10 m square with a crate on it, half a metre square.
/// let floor = [
///     [Vec3::new(-5.0, 0.0, -5.0), Vec3::new(5.0, 0.0, -5.0), Vec3::new(5.0, 0.0, 5.0)],
///     [Vec3::new(-5.0, 0.0, -5.0), Vec3::new(5.0, 0.0, 5.0), Vec3::new(-5.0, 0.0, 5.0)],
/// ];
/// let crate_ = Obstacle::cuboid(Vec3::new(-0.25, 0.0, -0.25), Vec3::new(0.25, 0.5, 0.25));
/// let mesh = Navmesh::derive(floor, &[crate_.clone().with_clearance(0.25)]);
/// assert_eq!(mesh.height_at(Vec2::new(2.0, 2.0)), Some(0.0));
/// // Within the crate's clearance, half a metre from its middle, nobody stands.
/// assert_eq!(mesh.height_at(Vec2::new(0.45, 0.0)), None);
/// assert!(crate_.keeps(Vec2::new(0.45, 0.0)));
/// assert_eq!(mesh.height_at(Vec2::new(0.55, 0.0)), Some(0.0));
/// ```
#[derive(Clone, Debug, PartialEq)]
pub struct Obstacle {
    /// Its footprint's corners, anticlockwise in X and Z, as a plane's x and y.
    hull: Vec<Vec2>,
    bottom: f32,
    top: f32,
    clearance: f32,
}

/// The convex hull of `points`, anticlockwise in X and Z: none, one or two points where they enclose nothing.
fn hull(points: impl IntoIterator<Item = Vec2>) -> Vec<Vec2> {
    let mut points: Vec<Vec2> = points
        .into_iter()
        .filter(|point| point.is_finite())
        .collect();
    points.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    points.dedup();
    if points.len() < 3 {
        return points;
    }
    // Andrew's monotone chain: the lower hull, then the upper, each turning only left.
    let mut hull: Vec<Vec2> = Vec::with_capacity(points.len() * 2);
    for pass in 0..2 {
        let start = hull.len();
        let ordered: Box<dyn Iterator<Item = &Vec2>> = if pass == 0 {
            Box::new(points.iter())
        } else {
            Box::new(points.iter().rev())
        };
        for &point in ordered {
            while hull.len() >= start + 2 {
                let (a, b) = (hull[hull.len() - 2], hull[hull.len() - 1]);
                if (b - a).perp_dot(point - a) > 0.0 {
                    break;
                }
                hull.pop();
            }
            hull.push(point);
        }
        // The last point of each pass is the first of the next.
        hull.pop();
    }
    hull
}

impl Obstacle {
    /// An obstacle whose footprint is the convex hull of `corners`, X and Z, from `bottom` up to `top`: corners that
    /// are not numbers are left out, and a footprint of one or two corners, a post or a rail, is cut about all the
    /// same.
    pub fn footprint(corners: impl IntoIterator<Item = Vec2>, bottom: f32, top: f32) -> Self {
        Self {
            hull: hull(corners),
            bottom: bottom.min(top),
            top: top.max(bottom),
            clearance: CLEARANCE,
        }
    }

    /// An upright box from its least corner to its greatest, as an environment's solids are declared.
    pub fn cuboid(low: Vec3, high: Vec3) -> Self {
        let (low, high) = (low.min(high), low.max(high));
        Self::footprint(
            [
                Vec2::new(low.x, low.z),
                Vec2::new(high.x, low.z),
                Vec2::new(high.x, high.z),
                Vec2::new(low.x, high.z),
            ],
            low.y,
            high.y,
        )
    }

    /// The obstacle, with ground cut `clearance` about it rather than [`CLEARANCE`]: none for one that is not a
    /// number or is less than nothing.
    pub fn with_clearance(mut self, clearance: f32) -> Self {
        self.clearance = if clearance > 0.0 { clearance } else { 0.0 };
        self
    }

    /// Its footprint's corners, anticlockwise in X and Z.
    pub fn hull(&self) -> &[Vec2] {
        &self.hull
    }

    /// How low and how high it reaches.
    pub fn bottom(&self) -> f32 {
        self.bottom
    }

    pub fn top(&self) -> f32 {
        self.top
    }

    /// How far about its footprint ground is cut.
    pub fn clearance(&self) -> f32 {
        self.clearance
    }

    /// The lines bounding what it cuts, seen from above: each a way out of it, of unit length, and how far along that
    /// way its edge is, so that a point `p` is cut where `way · p ≤ offset` for every one. Its footprint's edges and
    /// its box's, each moved out by its clearance. None for an obstacle without a corner.
    fn lines(&self) -> Vec<(Vec2, f32)> {
        let Some((low, high)) = self.bounds() else {
            return Vec::new();
        };
        let mut lines = vec![
            (Vec2::NEG_X, -low.x),
            (Vec2::X, high.x),
            (Vec2::NEG_Y, -low.y),
            (Vec2::Y, high.y),
        ];
        if self.hull.len() >= 3 {
            for (index, &from) in self.hull.iter().enumerate() {
                let to = self.hull[(index + 1) % self.hull.len()];
                // Anticlockwise, the outside of each edge is on its right.
                let way = Vec2::new(to.y - from.y, from.x - to.x).normalize_or_zero();
                lines.push((way, way.dot(from) + self.clearance));
            }
        }
        lines
    }

    /// The box about what it cuts, seen from above.
    fn bounds(&self) -> Option<(Vec2, Vec2)> {
        let first = *self.hull.first()?;
        let (low, high) = self.hull.iter().fold((first, first), |(low, high), point| {
            (low.min(*point), high.max(*point))
        });
        Some((low - self.clearance, high + self.clearance))
    }

    /// Whether `point`, seen from above, is within what it cuts, its edge included, at any height.
    pub fn keeps(&self, point: Vec2) -> bool {
        let lines = self.lines();
        !lines.is_empty() && lines.iter().all(|(way, offset)| way.dot(point) <= *offset)
    }

    /// Whether it reaches into the room of ground from `low` to `high`.
    fn reaches(&self, low: f32, high: f32) -> bool {
        self.bottom < high + HEADROOM && self.top > low
    }
}

/// Walkable ground as triangles, each at the heights it is drawn, indexed on a grid: what [`Navmesh::derive`] makes of
/// an environment's ground and obstacles, or [`Navmesh::from_triangles`] of triangles already walkable. A point is on
/// it where it is within a triangle, seen from above, edges included; where triangles overlap, as a track over the
/// meadow, the highest is the floor.
#[derive(Clone, PartialEq)]
pub struct Navmesh {
    triangles: Vec<[Vec3; 3]>,
    /// The least and greatest of every corner.
    low: Vec3,
    high: Vec3,
    /// The grid: its cells' size, how many along X and Z, and, for each cell, row by row along X, the triangles whose
    /// box reaches it, from `starts[cell]` to `starts[cell + 1]` in `entries`.
    cell: f32,
    columns: usize,
    rows: usize,
    starts: Vec<u32>,
    entries: Vec<u32>,
}

impl std::fmt::Debug for Navmesh {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Navmesh")
            .field("triangles", &self.triangles.len())
            .field("low", &self.low)
            .field("high", &self.high)
            .finish_non_exhaustive()
    }
}

/// Twice the area `points` enclose seen from above, positive anticlockwise in X and Z.
fn doubled_area(points: &[Vec3]) -> f32 {
    (0..points.len())
        .map(|index| {
            let (a, b) = (points[index], points[(index + 1) % points.len()]);
            a.x * b.z - b.x * a.z
        })
        .sum()
}

/// `polygon`, convex, cut by the line where `way · p = offset`, seen from above: what lies beyond it and what lies
/// within, each corner interpolated along the edge it cuts, so that it stays on the polygon's plane.
fn split(polygon: &[Vec3], way: Vec2, offset: f32) -> (Vec<Vec3>, Vec<Vec3>) {
    let (mut beyond, mut within) = (Vec::new(), Vec::new());
    let side = |point: Vec3| way.x * point.x + way.y * point.z - offset;
    for (index, &from) in polygon.iter().enumerate() {
        let to = polygon[(index + 1) % polygon.len()];
        let (a, b) = (side(from), side(to));
        if a >= 0.0 {
            beyond.push(from);
        }
        if a <= 0.0 {
            within.push(from);
        }
        if (a > 0.0 && b < 0.0) || (a < 0.0 && b > 0.0) {
            let crossing = from.lerp(to, a / (a - b));
            beyond.push(crossing);
            within.push(crossing);
        }
    }
    (beyond, within)
}

/// The pieces of `piece`, convex, beyond what `lines` bound, each convex, into `pieces`.
fn subtract(piece: Vec<Vec3>, lines: &[(Vec2, f32)], pieces: &mut Vec<Vec<Vec3>>) {
    let mut within = piece;
    for &(way, offset) in lines {
        let (beyond, rest) = split(&within, way, offset);
        if beyond.len() >= 3 && doubled_area(&beyond).abs() > 2.0 * SLIVER {
            pieces.push(beyond);
        }
        within = rest;
        if within.len() < 3 {
            return;
        }
    }
}

impl Navmesh {
    /// The walkable ground of `ground`, the triangles an environment draws as floor and marks as ground: those that
    /// face up, whichever way they wind, sloping no more than [`STEEPEST`], less what each of `obstacles` cuts that
    /// reaches into the room above them ([`Obstacle`]), as triangles at the heights the ground is drawn. A triangle
    /// whose corners are not numbers is left out, and only the first [`MOST_TRIANGLES`] are kept.
    pub fn derive(ground: impl IntoIterator<Item = [Vec3; 3]>, obstacles: &[Obstacle]) -> Self {
        let flattest = STEEPEST.cos();
        // Each obstacle, with the box about what it cuts and the lines bounding it.
        type Cutting<'a> = (&'a Obstacle, (Vec2, Vec2), Vec<(Vec2, f32)>);
        let cutting: Vec<Cutting> = obstacles
            .iter()
            .filter_map(|obstacle| Some((obstacle, obstacle.bounds()?, obstacle.lines())))
            .collect();
        let mut kept: Vec<[Vec3; 3]> = Vec::new();
        for [a, b, c] in ground {
            if kept.len() >= MOST_TRIANGLES {
                break;
            }
            if ![a, b, c].iter().all(|corner| corner.is_finite()) {
                continue;
            }
            let normal = (b - a).cross(c - a);
            let length = normal.length();
            if !length.is_normal() || normal.y.abs() < flattest * length {
                continue;
            }
            let low = a.min(b).min(c);
            let high = a.max(b).max(c);
            let (flat_low, flat_high) = (Vec2::new(low.x, low.z), Vec2::new(high.x, high.z));
            let mut pieces = vec![vec![a, b, c]];
            for (obstacle, (cut_low, cut_high), lines) in &cutting {
                if !(obstacle.reaches(low.y, high.y)
                    && cut_low.cmple(flat_high).all()
                    && flat_low.cmple(*cut_high).all())
                {
                    continue;
                }
                let mut next = Vec::with_capacity(pieces.len());
                for piece in pieces {
                    subtract(piece, lines, &mut next);
                }
                pieces = next;
                if pieces.is_empty() {
                    break;
                }
            }
            for piece in pieces {
                for index in 1..piece.len() - 1 {
                    let triangle = [piece[0], piece[index], piece[index + 1]];
                    if doubled_area(&triangle).abs() > 2.0 * SLIVER && kept.len() < MOST_TRIANGLES {
                        kept.push(triangle);
                    }
                }
            }
        }
        Self::indexed(kept)
    }

    /// `triangles`, as walkable as they are, indexed: those that face up and are numbers, whatever their slope, the
    /// first [`MOST_TRIANGLES`] of them.
    pub fn from_triangles(triangles: impl IntoIterator<Item = [Vec3; 3]>) -> Self {
        Self::indexed(
            triangles
                .into_iter()
                .filter(|[a, b, c]| {
                    [a, b, c].iter().all(|corner| corner.is_finite())
                        && (*b - *a).cross(*c - *a).y.abs() > 0.0
                })
                .take(MOST_TRIANGLES)
                .collect(),
        )
    }

    fn indexed(triangles: Vec<[Vec3; 3]>) -> Self {
        let (low, high) = triangles.iter().flatten().fold(
            (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)),
            |(low, high), corner| (low.min(*corner), high.max(*corner)),
        );
        if triangles.is_empty() {
            return Self {
                triangles,
                low: Vec3::ZERO,
                high: Vec3::ZERO,
                cell: CELL,
                columns: 0,
                rows: 0,
                starts: vec![0],
                entries: Vec::new(),
            };
        }
        let size = Vec2::new(high.x - low.x, high.z - low.z).max(Vec2::splat(SEAM));
        let cell = CELL.max((size.x * size.y / MOST_CELLS).sqrt());
        let columns = ((size.x / cell).ceil() as usize).clamp(1, MOST_CELLS as usize);
        let rows = ((size.y / cell).ceil() as usize).clamp(1, MOST_CELLS as usize / columns);
        let mut mesh = Self {
            triangles,
            low,
            high,
            cell,
            columns,
            rows,
            starts: vec![0; columns * rows + 1],
            entries: Vec::new(),
        };
        // Count each cell's triangles, then lay them out, each cell's after the one before.
        let spans: Vec<[usize; 4]> = mesh
            .triangles
            .iter()
            .map(|[a, b, c]| {
                let (from, to) = (a.min(*b).min(*c), a.max(*b).max(*c));
                let (first, last) = (
                    mesh.cell_of(Vec2::new(from.x, from.z) - SEAM),
                    mesh.cell_of(Vec2::new(to.x, to.z) + SEAM),
                );
                [first.0, last.0, first.1, last.1]
            })
            .collect();
        for &[x0, x1, z0, z1] in &spans {
            for row in z0..=z1 {
                for column in x0..=x1 {
                    mesh.starts[row * columns + column + 1] += 1;
                }
            }
        }
        for index in 1..mesh.starts.len() {
            mesh.starts[index] += mesh.starts[index - 1];
        }
        let mut filled = mesh.starts.clone();
        mesh.entries = vec![0; *mesh.starts.last().unwrap_or(&0) as usize];
        for (triangle, &[x0, x1, z0, z1]) in spans.iter().enumerate() {
            for row in z0..=z1 {
                for column in x0..=x1 {
                    let at = &mut filled[row * columns + column];
                    mesh.entries[*at as usize] = triangle as u32;
                    *at += 1;
                }
            }
        }
        mesh
    }

    /// The cell `point` is in, seen from above, or the nearest one to it.
    fn cell_of(&self, point: Vec2) -> (usize, usize) {
        let at = (point - Vec2::new(self.low.x, self.low.z)) / self.cell;
        (
            (at.x.floor().max(0.0) as usize).min(self.columns - 1),
            (at.y.floor().max(0.0) as usize).min(self.rows - 1),
        )
    }

    /// The triangles in the cell at `column` and `row`.
    fn in_cell(&self, column: usize, row: usize) -> impl Iterator<Item = &[Vec3; 3]> {
        let cell = row * self.columns + column;
        self.entries[self.starts[cell] as usize..self.starts[cell + 1] as usize]
            .iter()
            .map(|index| &self.triangles[*index as usize])
    }

    /// Its triangles, each at the heights it is drawn, for whoever draws it or paths across it.
    pub fn triangles(&self) -> &[[Vec3; 3]] {
        &self.triangles
    }

    /// Whether it has no ground at all.
    pub fn is_empty(&self) -> bool {
        self.triangles.is_empty()
    }

    /// The least and greatest of its corners, or `None` where it has none.
    pub fn bounds(&self) -> Option<(Vec3, Vec3)> {
        (!self.is_empty()).then_some((self.low, self.high))
    }

    /// The heights of its floors above and below `point`, seen from above, one for each triangle it is on.
    pub fn heights(&self, point: Vec2) -> impl Iterator<Item = f32> + '_ {
        let inside = !self.is_empty()
            && point.cmpge(Vec2::new(self.low.x, self.low.z) - SEAM).all()
            && point
                .cmple(Vec2::new(self.high.x, self.high.z) + SEAM)
                .all();
        let (column, row) = if inside { self.cell_of(point) } else { (0, 0) };
        inside
            .then(|| self.in_cell(column, row))
            .into_iter()
            .flatten()
            .filter_map(move |triangle| height_on(triangle, point))
    }

    /// The height of its highest floor at `point`, seen from above, or `None` where it has none.
    pub fn height_at(&self, point: Vec2) -> Option<f32> {
        self.heights(point).reduce(f32::max)
    }

    /// How far along the ray from `origin` along `direction` it first meets a triangle, no further than `before`:
    /// from above only, or from either side where `either_way`. In the ray's own measure, as far as `direction` is
    /// long; `None` where it meets none.
    pub fn meets(
        &self,
        origin: Vec3,
        direction: Vec3,
        before: f32,
        either_way: bool,
    ) -> Option<f32> {
        if self.is_empty() || !origin.is_finite() || !direction.is_finite() {
            return None;
        }
        // Where the ray is within the box about every triangle, a seam wider.
        let (mut near, mut far) = (0.0f32, before);
        for axis in 0..3 {
            let (low, high) = (self.low[axis] - SEAM, self.high[axis] + SEAM);
            if direction[axis].abs() < f32::EPSILON {
                if origin[axis] < low || origin[axis] > high {
                    return None;
                }
                continue;
            }
            let (a, b) = (
                (low - origin[axis]) / direction[axis],
                (high - origin[axis]) / direction[axis],
            );
            near = near.max(a.min(b));
            far = far.min(a.max(b));
        }
        if near > far {
            return None;
        }
        // Through the grid's cells in order along the ray, seen from above, from where it enters the box.
        let start = origin + direction * near;
        let (mut column, mut row) = self.cell_of(Vec2::new(start.x, start.z));
        // How far along the ray it next crosses into the next cell along `axis`, X or Z, from cell `index`, and how
        // far it goes through a cell along it.
        let crossing = |axis: usize, index: usize| -> (f32, f32) {
            let (along, from, base) = (direction[axis], origin[axis], self.low[axis]);
            if along > 0.0 {
                (
                    (base + (index + 1) as f32 * self.cell - from) / along,
                    self.cell / along,
                )
            } else if along < 0.0 {
                (
                    (base + index as f32 * self.cell - from) / along,
                    -self.cell / along,
                )
            } else {
                (f32::INFINITY, f32::INFINITY)
            }
        };
        let (mut next_x, step_x) = crossing(0, column);
        let (mut next_z, step_z) = crossing(2, row);
        let mut best: Option<f32> = None;
        loop {
            for triangle in self.in_cell(column, row) {
                if let Some(distance) = hit(triangle, origin, direction, either_way)
                    && distance >= 0.0
                    && distance <= before
                    && best.is_none_or(|best| distance < best)
                {
                    best = Some(distance);
                }
            }
            let exit = next_x.min(next_z);
            if exit >= far || best.is_some_and(|best| best <= exit) {
                return best;
            }
            if next_x <= next_z {
                if direction.x > 0.0 && column + 1 < self.columns {
                    column += 1;
                } else if direction.x < 0.0 && column > 0 {
                    column -= 1;
                } else {
                    return best;
                }
                next_x += step_x;
            } else {
                if direction.z > 0.0 && row + 1 < self.rows {
                    row += 1;
                } else if direction.z < 0.0 && row > 0 {
                    row -= 1;
                } else {
                    return best;
                }
                next_z += step_z;
            }
        }
    }
}

/// The height of `triangle` at `point`, seen from above, if `point` is on it, a seam outside its edges included: from
/// its first corner, so that a flat triangle gives its own height exactly.
fn height_on([a, b, c]: &[Vec3; 3], point: Vec2) -> Option<f32> {
    let flat = |corner: Vec3| Vec2::new(corner.x, corner.z);
    let (fa, fb, fc) = (flat(*a), flat(*b), flat(*c));
    let doubled = (fb - fa).perp_dot(fc - fa);
    // Each corner's weight, and how far outside the edge across from it the point may be, as a weight.
    let weights = [
        (fc - fb).perp_dot(point - fb) / doubled,
        (fa - fc).perp_dot(point - fc) / doubled,
        (fb - fa).perp_dot(point - fa) / doubled,
    ];
    let edges = [fc - fb, fa - fc, fb - fa];
    // A triangle with no area, seen from above, holds nothing.
    let inside = doubled != 0.0
        && weights
            .iter()
            .zip(edges)
            .all(|(weight, edge)| *weight >= -SEAM * edge.length() / doubled.abs());
    inside.then(|| a.y + weights[1] * (b.y - a.y) + weights[2] * (c.y - a.y))
}

/// How far along the ray from `origin` along `direction` it meets `triangle`, from above or, where `either_way`, from
/// either side, or `None`.
fn hit(triangle: &[Vec3; 3], origin: Vec3, direction: Vec3, either_way: bool) -> Option<f32> {
    let [a, b, c] = *triangle;
    let mut normal = (b - a).cross(c - a);
    if normal.y < 0.0 {
        normal = -normal;
    }
    let facing = normal.dot(direction);
    if facing == 0.0 || (!either_way && facing > 0.0) {
        return None;
    }
    let distance = normal.dot(a - origin) / facing;
    let at = origin + direction * distance;
    height_on(triangle, Vec2::new(at.x, at.z)).map(|_| distance)
}

#[cfg(test)]
mod tests;
