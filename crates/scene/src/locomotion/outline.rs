// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The outline of a walkable surface that is neither a rectangle nor round: a simple polygon in an environment's
//! horizontal plane, checked and cut into triangles once, as it is declared, and what tells whether a point is on it.
//! See `design/locomotion.md`.
use glam::{DVec2, Vec2};

/// The most corners an outline may have.
pub const MOST_POINTS: usize = 64;
/// How far from the environment's origin, on each axis, an outline's corners may be, in metres, as an environment's
/// arrival is bounded.
pub const FARTHEST: f32 = 1000.0;
/// How near a corner of an outline may come to an edge it does not end, in metres: nearer, the outline repeats a
/// corner, folds back on itself or touches itself.
pub const CLEARANCE: f32 = 0.001;
/// The least an outline may enclose, in square metres.
pub const SMALLEST_AREA: f32 = 1e-4;

/// Why an outline is refused, as [`crate::locomotion::Surface::polygon`] answers. Corners are counted from 0 in the
/// order they were given, and an edge is named by the corner it starts from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum OutlineError {
    /// It has fewer than three corners, or more than [`MOST_POINTS`].
    #[non_exhaustive]
    Count { points: usize },
    /// A corner is not a number, or is beyond [`FARTHEST`] of the environment's origin.
    #[non_exhaustive]
    Point { index: usize },
    /// Two of its edges cross.
    #[non_exhaustive]
    Crossing { edges: [usize; 2] },
    /// It encloses less than [`SMALLEST_AREA`], as corners in a line do.
    Flat,
    /// A corner is within [`CLEARANCE`] of an edge it does not end: a repeated corner, an edge folded back on itself,
    /// or the outline touching itself.
    #[non_exhaustive]
    Close { point: usize, edge: usize },
    /// It could not be cut into triangles.
    Tangled,
}

impl std::fmt::Display for OutlineError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            Self::Count { points } => write!(
                formatter,
                "an outline has from 3 to {MOST_POINTS} corners, not {points}"
            ),
            Self::Point { index } => write!(
                formatter,
                "corner {index} is not a number, or is beyond {FARTHEST} m of the origin"
            ),
            Self::Crossing { edges: [a, b] } => {
                write!(formatter, "the edges from corners {a} and {b} cross")
            }
            Self::Flat => write!(
                formatter,
                "the outline encloses less than {SMALLEST_AREA} m²"
            ),
            Self::Close { point, edge } => write!(
                formatter,
                "corner {point} is within {CLEARANCE} m of the edge from corner {edge}"
            ),
            Self::Tangled => write!(formatter, "the outline cannot be cut into triangles"),
        }
    }
}

impl std::error::Error for OutlineError {}

/// A simple polygon in an environment's horizontal plane, X and Z, convex or not, without holes: the outline of a
/// walkable surface, made by [`crate::locomotion::Surface::polygon`], which checks it and cuts it into triangles
/// once. It is kept in the coordinates it was declared in, wherever its surface is carried.
#[derive(Clone, Debug, PartialEq)]
pub struct Outline {
    /// Its corners, the way round that keeps what it encloses on the left of each edge, X and Z as a plane's x and y.
    points: Vec<Vec2>,
    /// The triangles it is cut into, each three of its corners.
    triangles: Vec<[u8; 3]>,
    /// The box around it.
    low: Vec2,
    high: Vec2,
}

/// Which side of the line from `a` through `b` the point `c` is on: positive to its left, negative to its right and
/// zero on it. In double precision, where the differences of single precision coordinates are exact and the products
/// nearly so: rounding can decide only whether a point within a hair of an edge, or exactly on a slanted one, is on
/// it, never which side a point any further away is on.
fn side(a: Vec2, b: Vec2, c: Vec2) -> f64 {
    let (a, b, c) = (a.as_dvec2(), b.as_dvec2(), c.as_dvec2());
    (b - a).perp_dot(c - a)
}

/// How far `point` is from the edge from `a` to `b`.
fn distance(a: Vec2, b: Vec2, point: Vec2) -> f64 {
    let (a, b, point): (DVec2, DVec2, DVec2) = (a.as_dvec2(), b.as_dvec2(), point.as_dvec2());
    let edge = b - a;
    // An edge with no length is its one end.
    let along = (point - a).dot(edge) / edge.length_squared().max(f64::MIN_POSITIVE);
    point.distance(a + edge * along.clamp(0.0, 1.0))
}

/// Twice what `points` enclose, positive where they keep it on their left.
fn doubled_area(points: &[Vec2]) -> f64 {
    edges(points)
        .map(|(_, a, b)| a.as_dvec2().perp_dot(b.as_dvec2()))
        .sum()
}

/// Each edge of `points`, the last back to the first, by the corner it starts from and its two ends.
fn edges(points: &[Vec2]) -> impl Iterator<Item = (usize, Vec2, Vec2)> {
    (0..points.len()).map(|index| (index, points[index], points[(index + 1) % points.len()]))
}

/// The triangles `points`, which keep what they enclose on their left, are cut into, by clipping ears: a corner whose
/// triangle with its neighbours lies within the outline, with no other corner in it or on it, is cut off, until a
/// triangle is left. A corner on the line between its neighbours is dropped without a triangle. `None` where no
/// corner can be cut, as for edges that cross.
fn triangulate(points: &[Vec2]) -> Option<Vec<[u8; 3]>> {
    let mut left: Vec<u8> = (0..points.len() as u8).collect();
    let mut triangles = Vec::with_capacity(points.len().saturating_sub(2));
    let at = |index: u8| points[usize::from(index)];
    while left.len() >= 3 {
        let count = left.len();
        let ear = (0..count).find_map(|index| {
            let corners = [
                left[(index + count - 1) % count],
                left[index],
                left[(index + 1) % count],
            ];
            let [a, b, c] = corners.map(at);
            let turn = side(a, b, c);
            if turn == 0.0 {
                return Some((index, None));
            }
            let clear = left.iter().all(|other| {
                let point = at(*other);
                corners.contains(other)
                    || side(a, b, point) < 0.0
                    || side(b, c, point) < 0.0
                    || side(c, a, point) < 0.0
            });
            (turn > 0.0 && clear).then_some((index, Some(corners)))
        });
        let (index, triangle) = ear?;
        triangles.extend(triangle);
        left.remove(index);
    }
    Some(triangles)
}

impl Outline {
    /// The outline through `points`, in order, either way round, if it is one a participant can be told to be on:
    /// from 3 to [`MOST_POINTS`] corners, each finite and within [`FARTHEST`], no edges crossing, enclosing at least
    /// [`SMALLEST_AREA`], and no corner within [`CLEARANCE`] of an edge it does not end.
    pub(crate) fn new(points: impl IntoIterator<Item = Vec2>) -> Result<Self, OutlineError> {
        let mut points: Vec<Vec2> = points.into_iter().collect();
        let count = points.len();
        if !(3..=MOST_POINTS).contains(&count) {
            return Err(OutlineError::Count { points: count });
        }
        if let Some(index) = points
            .iter()
            .position(|point| !(point.is_finite() && point.abs().max_element() <= FARTHEST))
        {
            return Err(OutlineError::Point { index });
        }
        // Edges that share a corner meet there; any others must not cross.
        for (first, a, b) in edges(&points) {
            for (second, c, d) in edges(&points).skip(first + 2) {
                if (second + 1) % count != first
                    && side(a, b, c) * side(a, b, d) < 0.0
                    && side(c, d, a) * side(c, d, b) < 0.0
                {
                    return Err(OutlineError::Crossing {
                        edges: [first, second],
                    });
                }
            }
        }
        let doubled = doubled_area(&points);
        if doubled.abs() < 2.0 * f64::from(SMALLEST_AREA) {
            return Err(OutlineError::Flat);
        }
        for (point, corner) in points.iter().enumerate() {
            for (edge, a, b) in edges(&points) {
                if point != edge
                    && point != (edge + 1) % count
                    && distance(a, b, *corner) < f64::from(CLEARANCE)
                {
                    return Err(OutlineError::Close { point, edge });
                }
            }
        }
        if doubled < 0.0 {
            points.reverse();
        }
        let (low, high) = points
            .iter()
            .fold((points[0], points[0]), |(low, high), point| {
                (low.min(*point), high.max(*point))
            });
        triangulate(&points)
            .map(|triangles| Self {
                points,
                triangles,
                low,
                high,
            })
            .ok_or(OutlineError::Tangled)
    }

    /// Its corners, in the coordinates it was declared in, X and Z, in order about it: the order they were given in,
    /// or its reverse.
    pub fn points(&self) -> &[Vec2] {
        &self.points
    }

    /// The triangles it is cut into, each by its three corners, in the coordinates it was declared in, X and Z: they
    /// cover exactly what it encloses, for whoever draws the surface. A corner in a straight line between its
    /// neighbours belongs to no triangle of its own.
    pub fn triangles(&self) -> impl Iterator<Item = [Vec2; 3]> + '_ {
        self.triangles
            .iter()
            .map(|triangle| triangle.map(|corner| self.points[usize::from(corner)]))
    }

    /// Whether `point`, X and Z in the coordinates it was declared in, is within it or on its edge: outside the box
    /// around it, it is not; within, it is where the outline winds about it, or passes through it.
    pub fn contains(&self, point: Vec2) -> bool {
        if !(point.cmpge(self.low).all() && point.cmple(self.high).all()) {
            return false;
        }
        let mut winding = 0;
        for (_, a, b) in edges(&self.points) {
            let side = side(a, b, point);
            if side == 0.0 && point.cmpge(a.min(b)).all() && point.cmple(a.max(b)).all() {
                return true;
            }
            if a.y <= point.y {
                if b.y > point.y && side > 0.0 {
                    winding += 1;
                }
            } else if b.y <= point.y && side < 0.0 {
                winding -= 1;
            }
        }
        winding != 0
    }
}

/// A crescent 4 m deep at its middle, as a balcony's: its outer edge half a circle of 8 m about the origin, towards
/// −Z, its inner edge an arc of 10 m about a point 6 m behind the origin, the two meeting at its horns, 8 m to
/// either side.
#[cfg(test)]
pub(crate) fn crescent() -> Vec<Vec2> {
    let arc = |centre: Vec2, radius: f32, from: f32, to: f32, steps: usize| {
        (0..=steps).map(move |step| {
            let angle = from + (to - from) * step as f32 / steps as f32;
            centre + radius * Vec2::new(-angle.sin(), -angle.cos())
        })
    };
    let quarter = std::f32::consts::FRAC_PI_2;
    let horn = (4.0_f32 / 3.0).atan();
    let outer = arc(Vec2::ZERO, 8.0, quarter, -quarter, 24);
    let inner = arc(Vec2::new(0.0, 6.0), 10.0, -horn, horn, 16);
    outer.chain(inner.skip(1).take(15)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn points(corners: &[(f32, f32)]) -> Vec<Vec2> {
        corners.iter().map(|(x, z)| Vec2::new(*x, *z)).collect()
    }

    const SQUARE: &[(f32, f32)] = &[(0.0, 0.0), (4.0, 0.0), (4.0, 4.0), (0.0, 4.0)];
    const PENTAGON: &[(f32, f32)] = &[(0.0, 0.0), (4.0, -1.0), (6.0, 2.0), (3.0, 5.0), (-1.0, 3.0)];
    /// An L: a square with its far right quarter cut away.
    const ELL: &[(f32, f32)] = &[
        (0.0, 0.0),
        (4.0, 0.0),
        (4.0, 2.0),
        (2.0, 2.0),
        (2.0, 4.0),
        (0.0, 4.0),
    ];
    /// A U: two arms either side of a slot.
    const YOU: &[(f32, f32)] = &[
        (0.0, 0.0),
        (6.0, 0.0),
        (6.0, 4.0),
        (4.0, 4.0),
        (4.0, 2.0),
        (2.0, 2.0),
        (2.0, 4.0),
        (0.0, 4.0),
    ];
    /// An arrowhead, hollow behind.
    const CHEVRON: &[(f32, f32)] = &[(0.0, 0.0), (2.0, 1.0), (4.0, 0.0), (2.0, 4.0)];
    /// A square with a corner in the middle of each side.
    const STRAIGHT: &[(f32, f32)] = &[
        (0.0, 0.0),
        (2.0, 0.0),
        (4.0, 0.0),
        (4.0, 2.0),
        (4.0, 4.0),
        (2.0, 4.0),
        (0.0, 4.0),
        (0.0, 2.0),
    ];

    /// Whether `point` is in or on one of the triangles `outline` is cut into.
    fn in_triangles(outline: &Outline, point: Vec2) -> bool {
        outline.triangles().any(|[a, b, c]| {
            side(a, b, point) >= 0.0 && side(b, c, point) >= 0.0 && side(c, a, point) >= 0.0
        })
    }

    /// Whether `corners` wind about `point`, by brute force: the angles their edges turn through, seen from it, come
    /// to a whole turn, rather than to none. Not for a point on an edge.
    fn wound(corners: &[Vec2], point: Vec2) -> bool {
        let turned: f64 = edges(corners)
            .map(|(_, a, b)| {
                let (a, b) = ((a - point).as_dvec2(), (b - point).as_dvec2());
                a.perp_dot(b).atan2(a.dot(b))
            })
            .sum();
        turned.abs() > std::f64::consts::PI
    }

    /// How far `point` is from the nearest edge of `corners`.
    fn clearance(corners: &[Vec2], point: Vec2) -> f64 {
        edges(corners)
            .map(|(_, a, b)| distance(a, b, point))
            .fold(f64::INFINITY, f64::min)
    }

    /// A grid, `step` apart, over the box around `corners` and 1 m beyond it.
    fn grid(corners: &[Vec2], step: f32) -> Vec<Vec2> {
        let low = corners.iter().copied().reduce(Vec2::min).unwrap() - 1.0;
        let high = corners.iter().copied().reduce(Vec2::max).unwrap() + 1.0;
        let across = ((high - low) / step).ceil();
        (0..=across.x as u32)
            .flat_map(|x| {
                (0..=across.y as u32).map(move |z| low + step * Vec2::new(x as f32, z as f32))
            })
            .collect()
    }

    /// Check `outline`, made from `corners`, against both references at every point of a grid, and its triangles
    /// against what it encloses; how many of the grid's points are on it.
    fn checked(outline: &Outline, corners: &[Vec2], step: f32) -> usize {
        let cut: f64 = outline
            .triangles()
            .map(|triangle| doubled_area(&triangle))
            .sum();
        let enclosed = doubled_area(corners).abs();
        assert!((cut - enclosed).abs() < 1e-9 * enclosed, "{cut} {enclosed}");
        let grid = grid(corners, step);
        for point in &grid {
            let on = outline.contains(*point);
            assert_eq!(on, in_triangles(outline, *point), "{point} in {corners:?}");
        }
        for point in grid
            .iter()
            .filter(|point| clearance(corners, **point) > 1e-6)
        {
            assert_eq!(outline.contains(*point), wound(corners, *point), "{point}");
        }
        grid.iter()
            .filter(|point| outline.contains(**point))
            .count()
    }

    #[test]
    fn outlines_hold_what_they_enclose_whichever_way_round_and_wherever_they_start() {
        // Each shape, with points on it and points off it: in a notch, a slot, a hollow, and beyond it.
        let crescent: Vec<(f32, f32)> = crescent().iter().map(|point| (point.x, point.y)).collect();
        type Corners<'a> = &'a [(f32, f32)];
        let shapes: [(Corners, Corners, Corners); 7] = [
            (
                SQUARE,
                &[(2.0, 2.0), (0.5, 3.5)],
                &[(-0.5, 2.0), (2.0, 4.5)],
            ),
            (
                PENTAGON,
                &[(3.0, 2.0), (5.5, 2.0)],
                &[(0.0, -0.5), (5.5, 4.0)],
            ),
            (
                ELL,
                &[(1.0, 1.0), (3.0, 1.0), (1.0, 3.0)],
                &[(3.0, 3.0), (2.5, 2.5), (5.0, 1.0)],
            ),
            (
                YOU,
                &[(1.0, 3.0), (5.0, 3.0), (3.0, 1.0)],
                &[(3.0, 3.0), (3.0, 2.5), (7.0, 1.0)],
            ),
            (
                CHEVRON,
                &[(2.0, 2.0), (0.5, 0.5), (3.5, 0.5)],
                &[(2.0, 0.5), (0.5, 2.0), (2.0, 4.5)],
            ),
            (
                STRAIGHT,
                &[(2.0, 2.0), (3.5, 0.5)],
                &[(4.5, 2.0), (2.0, -0.5)],
            ),
            (
                &crescent,
                &[(0.0, -6.0), (-7.5, -1.0), (7.5, -1.0), (0.0, -7.9)],
                &[
                    (0.0, -3.0),
                    (0.0, 0.0),
                    (-5.0, -0.5),
                    (0.0, -8.1),
                    (7.0, 1.0),
                ],
            ),
        ];
        for (corners, on, off) in shapes {
            let corners = points(corners);
            let mut reversed = corners.clone();
            reversed.reverse();
            let mut holds = Vec::new();
            for start in 0..corners.len() {
                for round in [&corners, &reversed] {
                    let mut round = round.clone();
                    round.rotate_left(start);
                    let outline = Outline::new(round.iter().copied()).expect("a simple outline");
                    assert_eq!(outline.points().len(), corners.len());
                    for (x, z) in on {
                        assert!(
                            outline.contains(Vec2::new(*x, *z)),
                            "{x}, {z} on {corners:?}"
                        );
                    }
                    for (x, z) in off {
                        assert!(
                            !outline.contains(Vec2::new(*x, *z)),
                            "{x}, {z} off {corners:?}"
                        );
                    }
                    holds.push(checked(&outline, &round, 0.125));
                }
            }
            // The same points of the grid, however it was given.
            assert!(holds[0] > 100, "{holds:?}");
            assert!(holds.iter().all(|held| *held == holds[0]), "{holds:?}");
        }
    }

    #[test]
    fn an_outline_s_edge_and_corners_are_on_it() {
        let ell = Outline::new(points(ELL)).unwrap();
        // The middle of each edge, each corner, the one that turns inwards among them, and the box's far corner,
        // which is in the notch.
        for (x, z) in [
            (2.0, 0.0),
            (4.0, 1.0),
            (3.0, 2.0),
            (2.0, 3.0),
            (1.0, 4.0),
            (0.0, 2.0),
        ] {
            assert!(ell.contains(Vec2::new(x, z)), "{x}, {z}");
        }
        for (x, z) in ELL {
            assert!(ell.contains(Vec2::new(*x, *z)), "{x}, {z}");
        }
        assert!(!ell.contains(Vec2::new(4.0, 4.0)));
        // The least beyond an edge is off it, and the least within is on it.
        assert!(!ell.contains(Vec2::new(4.0_f32.next_up(), 1.0)));
        assert!(!ell.contains(Vec2::new(2.0, 0.0_f32.next_down())));
        assert!(!ell.contains(Vec2::new(3.0, 2.0_f32.next_up())));
        assert!(!ell.contains(Vec2::new(2.0_f32.next_up(), 2.0_f32.next_up())));
        assert!(ell.contains(Vec2::new(4.0_f32.next_down(), 1.0)));
        assert!(ell.contains(Vec2::new(2.0_f32.next_down(), 2.0_f32.next_up())));
        // Beside it, level with an edge or a corner, and nonsense, are off it.
        assert!(!ell.contains(Vec2::new(3.0, 4.0)));
        assert!(!ell.contains(Vec2::new(5.0, 2.0)));
        assert!(!ell.contains(Vec2::new(-1.0, 0.0)));
        assert!(!ell.contains(Vec2::new(f32::NAN, 1.0)));
        assert!(!ell.contains(Vec2::new(1.0, f32::INFINITY)));
        // An edge that slants: a point on it, and the least beyond it.
        let slant = Outline::new(points(&[(0.0, 0.0), (4.0, 0.0), (0.0, 4.0)])).unwrap();
        assert!(slant.contains(Vec2::new(2.0, 2.0)));
        assert!(slant.contains(Vec2::new(1.0, 3.0)));
        assert!(!slant.contains(Vec2::new(2.0, 2.0_f32.next_up())));
        assert!(slant.contains(Vec2::new(2.0, 2.0_f32.next_down())));
        // On the line of an edge, but beyond its end, is off it.
        assert!(!slant.contains(Vec2::new(-1.0, 5.0)));
        let chevron = Outline::new(points(CHEVRON)).unwrap();
        assert!(chevron.contains(Vec2::new(2.0, 1.0)));
        assert!(!chevron.contains(Vec2::new(2.0, 1.0_f32.next_down())));
        assert!(!chevron.contains(Vec2::new(3.0, 0.5_f32.next_down())));
    }

    #[test]
    fn an_outline_that_is_not_simple_is_refused_saying_why() {
        let ring = |count: usize| -> Vec<Vec2> {
            (0..count)
                .map(|index| Vec2::from_angle(index as f32 / count as f32 * std::f32::consts::TAU))
                .collect()
        };
        let refused: [(Vec<Vec2>, OutlineError); 15] = [
            (Vec::new(), OutlineError::Count { points: 0 }),
            (points(&SQUARE[..2]), OutlineError::Count { points: 2 }),
            (ring(MOST_POINTS + 1), OutlineError::Count { points: 65 }),
            (ring(200), OutlineError::Count { points: 200 }),
            (
                points(&[(0.0, 0.0), (f32::NAN, 0.0), (4.0, 4.0)]),
                OutlineError::Point { index: 1 },
            ),
            (
                points(&[(0.0, 0.0), (4.0, 0.0), (4.0, f32::INFINITY)]),
                OutlineError::Point { index: 2 },
            ),
            (
                points(&[(-1000.5, 0.0), (4.0, 0.0), (4.0, 4.0)]),
                OutlineError::Point { index: 0 },
            ),
            // A bow, its edges crossing in the middle; and one whose ends cross.
            (
                points(&[(0.0, 0.0), (2.0, 2.0), (2.0, 0.0), (0.0, 2.0)]),
                OutlineError::Crossing { edges: [0, 2] },
            ),
            (
                points(&[(0.0, 0.0), (4.0, 0.0), (4.0, 4.0), (2.0, -1.0), (0.0, 4.0)]),
                OutlineError::Crossing { edges: [0, 2] },
            ),
            // Corners in a line, and a triangle a centimetre across.
            (
                points(&[(0.0, 0.0), (1.0, 0.0), (2.0, 0.0)]),
                OutlineError::Flat,
            ),
            (
                points(&[(0.0, 0.0), (0.01, 0.0), (0.0, 0.01)]),
                OutlineError::Flat,
            ),
            // A corner given twice, the first given again to close it, a notch under a millimetre wide, and a corner
            // that touches another edge.
            (
                points(&[(0.0, 0.0), (0.0, 0.0), (4.0, 0.0), (4.0, 4.0)]),
                OutlineError::Close { point: 0, edge: 1 },
            ),
            (
                points(&[(0.0, 0.0), (4.0, 0.0), (4.0, 4.0), (0.0, 0.0)]),
                OutlineError::Close { point: 0, edge: 2 },
            ),
            (
                points(&[
                    (0.0, 0.0),
                    (4.0, 0.0),
                    (4.0, 4.0),
                    (0.0, 4.0),
                    (0.0, 2.0004),
                    (2.0, 2.0),
                    (0.0, 1.9996),
                ]),
                OutlineError::Close { point: 4, edge: 5 },
            ),
            (
                points(&[(0.0, 0.0), (4.0, 0.0), (4.0, 4.0), (2.0, 0.0), (0.0, 4.0)]),
                OutlineError::Close { point: 3, edge: 0 },
            ),
        ];
        for (corners, why) in refused {
            assert_eq!(
                Outline::new(corners.iter().copied()),
                Err(why),
                "{corners:?}"
            );
        }
        // The most corners there may be are taken.
        assert_eq!(
            Outline::new(ring(MOST_POINTS)).unwrap().triangles().count(),
            62
        );
    }

    #[test]
    fn a_narrow_outline_and_corners_in_a_line_are_taken() {
        // A sliver 2 mm deep: narrower than anyone would aim at, and still an outline.
        let sliver = points(&[(0.0, 0.0), (1.0, 0.0), (0.5, 0.002)]);
        let outline = Outline::new(sliver.iter().copied()).unwrap();
        assert!(outline.contains(Vec2::new(0.5, 0.001)));
        assert!(!outline.contains(Vec2::new(0.5, 0.003)));
        assert_eq!(outline.triangles().count(), 1);
        // A square with a corner in the middle of each side is the square; a corner between two others in a line is
        // cut off with a triangle where that leaves an outline, and otherwise needs none.
        let straight = Outline::new(points(STRAIGHT)).unwrap();
        let square = Outline::new(points(SQUARE)).unwrap();
        for point in grid(&points(SQUARE), 0.25) {
            assert_eq!(straight.contains(point), square.contains(point), "{point}");
        }
        assert_eq!(straight.triangles().count(), 6);
        let mut middle = points(STRAIGHT);
        middle.rotate_left(1);
        assert_eq!(Outline::new(middle).unwrap().triangles().count(), 5);
        assert_eq!(square.triangles().count(), 2);
        // Given the other way round, it is kept the way that has it on the left.
        let mut clockwise = points(SQUARE);
        clockwise.reverse();
        assert_eq!(
            Outline::new(clockwise.iter().copied()).unwrap().points(),
            clockwise.iter().rev().copied().collect::<Vec<_>>()
        );
        assert_eq!(square.points(), points(SQUARE));
    }

    #[test]
    fn many_outlines_agree_with_brute_force_at_every_point_of_a_grid() {
        // Outlines seen whole from their middle, so simple, their corners at radii and bearings from a fixed
        // sequence: each agrees with its triangles, and with the angles its edges turn through, all over a grid.
        let mut state = 0x2545_f491_4f6c_dd1d_u64;
        let mut next = move || {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (state >> 40) as f32 / (1_u64 << 24) as f32
        };
        let mut held = 0;
        for count in [3, 4, 5, 7, 12, 23, 40, MOST_POINTS] {
            for _ in 0..4 {
                let middle = Vec2::new(next() * 40.0 - 20.0, next() * 40.0 - 20.0);
                let corners: Vec<Vec2> = (0..count)
                    .map(|index| {
                        let bearing = (index as f32 + 0.6 * next()) / count as f32;
                        let radius = 2.0 + 3.0 * next();
                        middle + radius * Vec2::from_angle(bearing * std::f32::consts::TAU)
                    })
                    .collect();
                let outline =
                    Outline::new(corners.iter().copied()).expect("seen whole from its middle");
                held += checked(&outline, &corners, 0.25);
            }
        }
        assert!(held > 5_000, "{held}");
    }

    #[test]
    fn edges_that_cross_cannot_be_cut_into_triangles() {
        assert_eq!(
            triangulate(&points(&[(0.0, 0.0), (2.0, 2.0), (2.0, 0.0), (0.0, 2.0)])),
            None
        );
        assert_eq!(
            triangulate(&points(SQUARE)),
            Some(vec![[3, 0, 1], [3, 1, 2]])
        );
    }

    #[test]
    fn each_refusal_says_what_is_wrong() {
        let refusals = [
            OutlineError::Count { points: 2 },
            OutlineError::Point { index: 1 },
            OutlineError::Crossing { edges: [0, 2] },
            OutlineError::Flat,
            OutlineError::Close { point: 3, edge: 0 },
            OutlineError::Tangled,
        ];
        let said: Vec<String> = refusals.iter().map(ToString::to_string).collect();
        assert_eq!(
            said,
            [
                "an outline has from 3 to 64 corners, not 2",
                "corner 1 is not a number, or is beyond 1000 m of the origin",
                "the edges from corners 0 and 2 cross",
                "the outline encloses less than 0.0001 m²",
                "corner 3 is within 0.001 m of the edge from corner 0",
                "the outline cannot be cut into triangles",
            ]
        );
        let error: Box<dyn std::error::Error> = Box::new(OutlineError::Flat);
        assert!(error.source().is_none());
    }
}
