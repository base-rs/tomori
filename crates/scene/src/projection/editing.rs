// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Editing walls with the grip: taking hold of a wall by pointing at its board or an empty slot, the handles that then
//! appear on it, small and grabbed with the grip, to size its slots, give it columns and rows, step them one at a time
//! and bend it, carrying a wall someone added as a panel is carried, and removing one, as an owner, with B. Each change
//! is shown at once, as the wall would be, and sent to the server when it is let go, shown so until the server's
//! projection agrees or [`super::PENDING`] passes. See `design/projection.md`.
use std::time::{Duration, Instant};

use glam::{Mat4, Quat, Vec2, Vec3};
use tomori_protocol::{Grid, Pose, Projection, Wall, WallKind, WallShape, wall as layout};

use super::{Controls, FRAME, HIGHLIGHT, HINT_ANGLE, PENDING, THROW, slot_unit};
use crate::{
    board::{model, pose},
    canvas::Canvas,
    controller_ui::Sheet,
    manipulation::Manipulation,
    pointing::Ray,
    recording::facing,
    transcript::{BRIGHT, TEXT},
    ui_text::{Row, Style, cut},
};

/// The step a corner sizes a wall's slots' text by, as a proportion of a standard slot's.
pub const SCALE_STEP: f32 = 0.05;

/// The shapes of slot a corner dragged snaps to as it nears them, as their width over their height, and how each is
/// named as it is dragged.
pub const ASPECT_SNAPS: [(f32, &str); 6] = [
    (1.0, "1:1"),
    (4.0 / 3.0, "4:3"),
    (3.0 / 4.0, "3:4"),
    (16.0 / 9.0, "16:9"),
    (9.0 / 16.0, "9:16"),
    (layout::STANDARD_ASPECT, "standard"),
];

/// How near one of [`ASPECT_SNAPS`] a slot's shape snaps to it, as the logarithm of their ratio: within about 3.5%.
const ASPECT_SNAP: f32 = 0.035;

/// How far the hand moves towards the participant, from where it took hold of the knob, to bend a wall from flat to as
/// tightly as it may curve, in metres; moving back flattens it as evenly.
pub const BEND_TRAVEL: f32 = 0.4;

/// How little a wall bends, as a proportion of the most it may, before it snaps flat.
const FLAT_BEND: f32 = 0.06;

/// How near where it took hold of the knob the hand is, in metres, for the wall to be bent as it was.
const BEND_REST: f32 = 0.005;

/// How large a handle is drawn, as the angle it takes from the eyes at the far side of its wall, whatever its distance,
/// so that it is as easy to hold with the ray near and far: about 1.7°.
pub const HANDLE_ANGLE: f32 = 0.03;

/// How far apart the middles of neighbouring handles stand, as seen from the eyes, in handles: a gap of most of a handle
/// between them, about 1.4°.
pub const SPACING: f32 = 1.8;

/// How near the ray passes a handle's middle to meet it, as a proportion of its size: never as far as halfway to its
/// neighbour's, so that of two handles the ray meets at most one, and the nearest middle is taken.
const HANDLE_REACH: f32 = 0.8;

/// The least a wall seen obliquely is taken to be foreshortened, as the sine of the angle it is seen at, so that the
/// handles of a wall seen nearly edge on do not spread without bound.
const OBLIQUE: f32 = 0.35;

/// How long a press of B asking to remove a wall that holds panels waits for another to confirm it.
pub const CONFIRM: Duration = Duration::from_secs(4);

/// How far beyond a wall's edges its handles' middles stand, in handles.
const OUTSET: f32 = 0.9;

/// The most columns of the Tomori wall, in its one row, as the server allows.
const TOMORI_COLUMNS: u32 = 6;

/// Which of a wall's grids a stepper steps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    Columns,
    Rows,
}

/// A handle on a wall taken hold of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handle {
    /// A corner, left or right (-1 or 1) and below or above (-1 or 1): sizes the wall's slots, their width and height
    /// apart, so that they take any shape.
    Corner(i8, i8),
    /// The right edge: gives the wall more or fewer columns.
    Columns,
    /// The bottom edge: gives the wall more or fewer rows.
    Rows,
    /// A stepper: one column or row fewer (-1) or more (1).
    Step(Axis, i8),
    /// The knob at its top middle: pulled towards the participant from where it was taken hold of, it bends the wall
    /// around them, and pushed back, flattens it, flat near where it bends least.
    Bend,
}

/// What a participant may do with walls in the joined space.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rights {
    /// Adjust walls: a participant, in a space whose server takes `SetWall`.
    pub adjust: bool,
    /// Remove walls: an owner, as well.
    pub remove: bool,
}

/// A change to a wall a participant made, to send: each left out stays as it is.
#[derive(Clone, Debug, PartialEq)]
pub struct Change {
    pub wall: String,
    pub grid: Option<Grid>,
    pub scale: Option<f32>,
    /// Its slots' width over their height, `0` for a standard slot's.
    pub aspect: Option<f32>,
    pub shape: Option<WallShape>,
    pub pose: Option<Pose>,
}

/// A handle being dragged, and the wall as it was when it was taken hold of.
#[derive(Clone, Debug, PartialEq)]
struct Drag {
    handle: Handle,
    start: Wall,
    /// Where the ray met the wall's plane as it was taken hold of, in the wall's own frame, which a corner's drag is
    /// measured against.
    grab: Vec2,
    /// Where the ray began, the hand, as it was taken hold of, how far along it the handle was and where that is, which
    /// the knob's drag is measured from.
    origin: Vec3,
    along: f32,
    point: Vec3,
}

/// A wall someone added, carried along the ray, as a panel is: the point it was taken hold of stays on the ray, and it
/// keeps facing the way it faced, turned only as the stick turns it, or squared up to face the participant.
#[derive(Clone, Debug, PartialEq)]
struct Carry {
    manipulation: Manipulation,
    /// The way it faced as it was taken hold of, or squared up.
    facing: Quat,
    /// Where it was taken hold of, in its own frame.
    grabbed: Vec3,
    model: Mat4,
}

/// The walls being edited across frames.
#[derive(Clone, Debug, Default)]
pub struct Editor {
    /// The wall taken hold of, whose handles show.
    wall: Option<String>,
    drag: Option<Drag>,
    carry: Option<Carry>,
    /// The wall as the participant is changing it, drawn in place of the server's.
    preview: Option<Wall>,
    /// The wall as it was changed and sent, drawn until the server's agrees or [`PENDING`] passes.
    pending: Option<(Wall, Instant)>,
    /// The handle the ray meets.
    hovered: Option<Handle>,
    /// When B asked to remove the wall carried, which holds panels, so that another press confirms it.
    confirm: Option<Instant>,
}

/// What the editor needs this frame.
#[derive(Clone, Copy, Debug)]
pub struct Input<'a> {
    pub projection: &'a Projection,
    pub ray: Option<Ray>,
    pub pressed: bool,
    pub released: bool,
    /// Whether the ray and grip act on what is projected.
    pub active: bool,
    /// Whether the ray points at what is projected, not while a teleport is aimed.
    pub pointing: bool,
    /// How far along the ray something else the grip would take is, such as a card, which a handle must be nearer than.
    pub nearer: f32,
    /// How far along the ray the nearest panel is, which a wall's board or slot must be nearer than to be taken hold of.
    pub panel: f32,
    pub stick: Vec2,
    pub straighten: bool,
    pub viewer: Vec3,
    pub rights: Rights,
    pub controls: Controls,
    pub now: Instant,
    pub dt: f32,
}

/// What the editor did this frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Output {
    /// It took the grip's press.
    pub took: bool,
    /// The handle the ray meets changed, or the wall carried turned, to tick.
    pub ticked: bool,
    /// A change to send.
    pub change: Option<Change>,
    /// Where the ray ends, on a handle or the wall carried.
    pub end: Option<Vec3>,
    /// The handles and the hint.
    pub sheets: Vec<Sheet>,
    /// How far along the ray a handle is, which the ray meets before anything behind it.
    pub reach: Option<f32>,
    /// How far along the ray, and where, it ends on what the editor has: a handle dragged or a wall carried, which it
    /// ends on whatever else it meets, or else the handle or the wall it meets, where the participant may take hold of
    /// one.
    pub hit: Option<crate::pointing::End>,
}

/// Where `ray` meets the plane of a wall at `frame`, in the wall's own frame, unscaled, if it does, in front of the ray.
fn on_plane(frame: Mat4, ray: &Ray) -> Option<Vec3> {
    let local = frame.inverse();
    let origin = local.transform_point3(ray.origin);
    let direction = local.transform_vector3(ray.direction);
    if direction.z.abs() < 1e-5 {
        return None;
    }
    let along = -origin.z / direction.z;
    (along > 0.0).then(|| origin + direction * along)
}

/// The corners of `wall` in its own frame: its board's, or, curved, its corner slots' outer corners.
fn corners(wall: &Wall) -> [Vec3; 4] {
    if let Some((width, height)) = wall.board() {
        let (x, y) = (width / 2.0, height / 2.0);
        return [
            Vec3::new(-x, y, 0.0),
            Vec3::new(x, y, 0.0),
            Vec3::new(-x, -y, 0.0),
            Vec3::new(x, -y, 0.0),
        ];
    }
    let grid = wall.grid();
    let slots = super::slot_frames(wall, Mat4::IDENTITY);
    let (width, height) = slot_unit(wall);
    let corner = |column: u32, row: u32, x: f32, y: f32| {
        let slot = grid.slot(column, row).unwrap_or_default() as usize;
        slots[slot].transform_point3(Vec3::new(x * width / 2.0, y * height / 2.0, 0.0))
    };
    let (last, bottom) = (grid.columns - 1, grid.rows - 1);
    [
        corner(0, 0, -1.0, 1.0),
        corner(last, 0, 1.0, 1.0),
        corner(0, bottom, -1.0, -1.0),
        corner(last, bottom, 1.0, -1.0),
    ]
}

/// Which handles `wall` has, for a participant who may adjust it: the Tomori wall's grid only in its one row, and its
/// slots their own size.
fn handles(wall: &Wall) -> Vec<Handle> {
    let mut handles = vec![
        Handle::Columns,
        Handle::Step(Axis::Columns, -1),
        Handle::Step(Axis::Columns, 1),
        Handle::Bend,
    ];
    if wall.kind != WallKind::Tomori {
        handles.extend([
            Handle::Corner(-1, 1),
            Handle::Corner(1, 1),
            Handle::Corner(-1, -1),
            Handle::Corner(1, -1),
            Handle::Rows,
            Handle::Step(Axis::Rows, -1),
            Handle::Step(Axis::Rows, 1),
        ]);
    }
    handles
}

/// Where each of `wall`'s handles stands, the wall standing at `frame` and seen from `viewer`, in the space, and how large
/// a handle is drawn, the same for every one: [`HANDLE_ANGLE`] from the eyes at the wall's farthest corner, so that
/// none is smaller. Its corners stand diagonally beyond its corners; the knob above its top edge, at its middle, and the
/// steppers of its columns along that edge towards its right corner; the handle of its columns beside its right edge, at
/// its middle, and the steppers of its rows along that edge towards its bottom corner; and the handle of its rows below
/// its bottom edge, at its middle. Each handle's middle is at least [`SPACING`] handles from every other's, as seen from
/// the viewer, however small the wall, whatever its shape and however obliquely it is seen: where an edge has no room
/// for them, the knob and the steppers move along it, then to a row or column of their own beyond it.
pub fn handle_points(wall: &Wall, frame: Mat4, viewer: Vec3) -> (Vec<(Handle, Vec3)>, f32) {
    // Laid out once at the size its corners take, and again at the size its farthest handle then takes, so that none is
    // smaller than its angle.
    let (points, _) = lay_out_handles(wall, frame, viewer, None);
    let far = points
        .iter()
        .map(|(_, at)| at.distance(viewer))
        .fold(0.3, f32::max);
    lay_out_handles(wall, frame, viewer, Some(far))
}

/// The handles of `wall` as [`handle_points`] lays them out, sized for a handle `far` from the viewer, or else for its
/// farthest corner.
fn lay_out_handles(
    wall: &Wall,
    frame: Mat4,
    viewer: Vec3,
    far: Option<f32>,
) -> (Vec<(Handle, Vec3)>, f32) {
    // The wall measured along itself, about its middle: a flat wall's board, or a curved wall's slots along their arc,
    // where a handle beyond its edge continues along the arc's last direction.
    let (width, height) = wall.size();
    let (right, top) = (width / 2.0, height / 2.0);
    let (left, bottom) = (-right, -top);
    let radius = wall.radius();
    let place = |x: f32, y: f32| match radius {
        None => Vec3::new(x, y, 0.0),
        Some(radius) => {
            let on = x.clamp(left, right);
            let angle = on / radius;
            let tangent = Vec3::new(angle.cos(), 0.0, angle.sin());
            Vec3::new(radius * angle.sin(), y, radius * (1.0 - angle.cos())) + tangent * (x - on)
        }
    };
    let tangent = |x: f32| {
        let angle = radius.map_or(0.0, |radius| x.clamp(left, right) / radius);
        frame.transform_vector3(Vec3::new(angle.cos(), 0.0, angle.sin()))
    };
    let samples = [
        (left, top),
        (right, top),
        (left, bottom),
        (right, bottom),
        (0.0, 0.0),
    ];
    let far = samples
        .iter()
        .map(|(x, y)| frame.transform_point3(place(*x, *y)).distance(viewer))
        .fold(far.unwrap_or(0.3), f32::max);
    let size = HANDLE_ANGLE * far;
    // A handle's worth along the wall and up it, longer as the wall is seen more obliquely, where it is seen most
    // obliquely, so that the handles stand as far apart to the eye wherever they are.
    let least = |axis: &dyn Fn(f32) -> Vec3| {
        samples
            .iter()
            .map(|(x, y)| {
                let towards =
                    (frame.transform_point3(place(*x, *y)) - viewer).normalize_or(Vec3::NEG_Z);
                let axis = axis(*x).normalize_or(Vec3::X);
                (1.0 - axis.dot(towards).powi(2)).max(0.0).sqrt()
            })
            .fold(1.0, f32::min)
            .max(OBLIQUE)
    };
    let up = frame.transform_vector3(Vec3::Y);
    let (u, v) = (size / least(&tangent), size / least(&|_| up));
    let at = place;
    let (xl, xr) = (left - OUTSET * u, right + OUTSET * u);
    let (yt, yb) = (top + OUTSET * v, bottom - OUTSET * v);
    let (pu, pv) = (SPACING * u, SPACING * v);
    let between = |preferred: f32, low: f32, high: f32| preferred.max(low).min(high);
    // Along the top: the corners, the knob at the middle where it has room, and the steppers towards the right corner.
    let room = xr - xl;
    let (knob, fewer, more) = if room >= 4.0 * pu - 1e-6 {
        (
            at(between(0.0, xl + pu, xr - 3.0 * pu), yt),
            at(xr - 2.0 * pu, yt),
            at(xr - pu, yt),
        )
    } else if room >= 2.0 * pu - 1e-6 {
        (
            at(between(0.0, xl + pu, xr - pu), yt),
            at(xr - pu, yt + pv),
            at(xr, yt + pv),
        )
    } else {
        (
            at(xr - 2.0 * pu, yt + pv),
            at(xr - pu, yt + pv),
            at(xr, yt + pv),
        )
    };
    // Down the right: the handle of its columns at the middle, and the steppers of its rows towards the bottom corner.
    let span = yt - yb;
    let centre = (top + bottom) / 2.0;
    let beyond = xr + pu;
    let tomori = wall.kind == WallKind::Tomori;
    let (columns, row_fewer, row_more) = if tomori {
        let columns = if span >= 2.0 * pv - 1e-6 {
            at(xr, between(centre, yb + pv, yt - pv))
        } else {
            at(beyond, centre)
        };
        (columns, Vec3::ZERO, Vec3::ZERO)
    } else if span >= 4.0 * pv - 1e-6 {
        (
            at(xr, between(centre, yb + 3.0 * pv, yt - pv)),
            at(xr, yb + 2.0 * pv),
            at(xr, yb + pv),
        )
    } else if span >= 2.0 * pv - 1e-6 {
        (
            at(xr, between(centre, yb + pv, yt - pv)),
            at(beyond, yb + pv),
            at(beyond, yb),
        )
    } else {
        (
            at(beyond, yb + 2.0 * pv),
            at(beyond, yb + pv),
            at(beyond, yb),
        )
    };
    // Along the bottom, the handle of its rows at the middle, or below where there is no room.
    let rows = if room >= 2.0 * pu - 1e-6 {
        at(between(0.0, xl + pu, xr - pu), yb)
    } else {
        at(0.0, yb - pv)
    };
    let local = handles(wall).into_iter().map(|handle| {
        let point = match handle {
            Handle::Corner(x, y) => at(if x < 0 { xl } else { xr }, if y > 0 { yt } else { yb }),
            Handle::Columns => columns,
            Handle::Rows => rows,
            Handle::Step(Axis::Columns, step) => {
                if step < 0 {
                    fewer
                } else {
                    more
                }
            }
            Handle::Step(Axis::Rows, step) => {
                if step < 0 {
                    row_fewer
                } else {
                    row_more
                }
            }
            Handle::Bend => knob,
        };
        (handle, frame.transform_point3(point))
    });
    let mut points: Vec<(Handle, Vec3)> = local.collect();
    // A wall curved so far that it folds as seen from the side may still bring two together: the one that is not a
    // corner moves on, beyond its edge, until none overlaps.
    let outward = |handle: Handle| match handle {
        Handle::Bend | Handle::Step(Axis::Columns, _) => up.normalize_or(Vec3::Y) * pv,
        Handle::Rows => -up.normalize_or(Vec3::Y) * pv,
        _ => tangent(right).normalize_or(Vec3::X) * pu,
    };
    let rank = |handle: Handle| match handle {
        Handle::Corner(..) => 0,
        Handle::Columns | Handle::Rows => 1,
        Handle::Bend => 2,
        Handle::Step(..) => 3,
    };
    for _ in 0..8 {
        let mut moved = false;
        for first in 0..points.len() {
            for second in first + 1..points.len() {
                let (a, b) = (points[first], points[second]);
                if apart(a.1, b.1, size, viewer, frame.transform_point3(Vec3::ZERO)) {
                    continue;
                }
                // Corners are always apart, standing beyond the wall's corners at least the spacing apart.
                let moving = if rank(a.0) >= rank(b.0) {
                    first
                } else {
                    second
                };
                let by = outward(points[moving].0);
                points[moving].1 += by;
                moved = true;
            }
        }
        if !moved {
            break;
        }
    }
    (points, size)
}

/// Whether handles `size` across, with their middles at `a` and `b`, stand apart as seen from `viewer`, looking towards
/// `middle`: their faces at least half a handle apart, across or up, as the eyes see them.
fn apart(a: Vec3, b: Vec3, size: f32, viewer: Vec3, middle: Vec3) -> bool {
    let ahead = (middle - viewer).normalize_or(Vec3::NEG_Z);
    let right = ahead.cross(Vec3::Y).normalize_or(Vec3::X);
    let up = right.cross(ahead);
    let seen = |point: Vec3| {
        let to = point - viewer;
        let distance = to.length().max(1e-3);
        let to = to / distance;
        (
            Vec2::new(
                to.dot(right).atan2(to.dot(ahead)),
                to.dot(up).clamp(-1.0, 1.0).asin(),
            ),
            size / distance,
        )
    };
    let ((a, across), (b, other)) = (seen(a), seen(b));
    let reach = (across + other) / 2.0 * 1.5;
    (a.x - b.x).abs() >= reach || (a.y - b.y).abs() >= reach
}

/// The handle of `points`, each `size` across, that `ray` meets, and how far along it: of those it passes within
/// [`HANDLE_REACH`] of a handle's size of, in front of it, the one whose middle it passes nearest, as an angle from where
/// it begins, so that of two close together the one aimed at is taken, not the first.
fn nearest_handle(points: &[(Handle, Vec3)], size: f32, ray: &Ray) -> Option<(Handle, f32)> {
    points
        .iter()
        .filter_map(|(handle, at)| {
            let along = (*at - ray.origin).dot(ray.direction);
            let off = (ray.at(along) - *at).length();
            (along > 0.0 && off <= size * HANDLE_REACH).then_some((*handle, along, off / along))
        })
        .min_by(|a, b| a.2.total_cmp(&b.2))
        .map(|(handle, along, _)| (handle, along))
}

/// Where `handle` of a wall like `wall`, standing at `frame`, is drawn, seen from `viewer`, and how large, as
/// [`handle_points`] lays them out, or the wall's middle for a handle it does not have.
pub fn handle_point(wall: &Wall, frame: Mat4, handle: Handle, viewer: Vec3) -> (Vec3, f32) {
    let (points, size) = handle_points(wall, frame, viewer);
    let at = points
        .into_iter()
        .find(|(shown, _)| *shown == handle)
        .map_or(frame.transform_point3(Vec3::ZERO), |(_, at)| at);
    (at, size)
}

/// What `handle` shows.
fn glyph(handle: Handle) -> &'static str {
    match handle {
        Handle::Corner(..) => " ",
        Handle::Columns => "↔",
        Handle::Rows => "↕",
        Handle::Step(_, -1) => "-",
        Handle::Step(..) => "+",
        Handle::Bend => "●",
    }
}

/// A handle's face, `size` across, highlighted while the ray meets it or it is dragged.
fn handle_face(handle: Handle, size: f32, highlighted: bool) -> Canvas {
    let mut canvas = Canvas::new(1, 1.0);
    // About square, so that it is as easy to meet across as down.
    canvas.padding = (4, 2);
    canvas.background = if highlighted {
        [0.1, 0.3, 0.36, 0.95]
    } else {
        [0.02, 0.04, 0.07, 0.9]
    };
    canvas.border = Some(if highlighted { HIGHLIGHT } else { FRAME });
    canvas.rows = vec![Row::text(glyph(handle), Style::bold(BRIGHT), 1)];
    let (width, _) = canvas.size_in_pixels();
    canvas.pixel = size / width as f32;
    canvas
}

/// `wall` with `grid`, `scale`, its slots' `aspect` and `radius`, its slots laid out so.
fn laid_out(wall: &Wall, grid: Grid, scale: f32, aspect: Option<f32>, radius: Option<f32>) -> Wall {
    let shape = radius.map_or(WallShape::Grid, |radius| WallShape::Arc { radius });
    wall.clone()
        .with_shape(shape)
        .with_grid(grid)
        .with_scale(scale)
        .with_aspect(aspect)
}

/// `wall` as it is, but for `grid`, its slots laid out so.
fn with_grid(wall: &Wall, grid: Grid) -> Wall {
    laid_out(wall, grid, wall.scale(), wall.aspect, wall.radius())
}

/// The size of text and shape that make a wall's slots `width` by `height` metres, as near as they may be: the shape
/// within [`layout::ASPECTS`], snapped to the nearest of [`ASPECT_SNAPS`] within [`ASPECT_SNAP`] of it, or else to a
/// hundredth, and the text as large as the narrower of the two against a standard slot, in steps of [`SCALE_STEP`]
/// within [`layout::SCALES`]; with the name of the shape snapped to, if it was. A standard slot's shape is `None`.
pub fn slot_shape(width: f32, height: f32) -> (f32, Option<f32>, Option<&'static str>) {
    let (width, height) = (width.max(1e-3), height.max(1e-3));
    let aspect = (width / height).clamp(layout::ASPECTS.0, layout::ASPECTS.1);
    let snapped = ASPECT_SNAPS
        .iter()
        .min_by(|a, b| {
            (aspect / a.0)
                .ln()
                .abs()
                .total_cmp(&(aspect / b.0).ln().abs())
        })
        .filter(|(shape, _)| (aspect / shape).ln().abs() < ASPECT_SNAP);
    let aspect = snapped.map_or((aspect * 100.0).round() / 100.0, |(shape, _)| *shape);
    let standard = layout::SLOT_SIZE;
    let scale = if aspect >= layout::STANDARD_ASPECT {
        height / standard.1
    } else {
        width / standard.0
    };
    let scale =
        ((scale / SCALE_STEP).round() * SCALE_STEP).clamp(layout::SCALES.0, layout::SCALES.1);
    let shape = (aspect != layout::STANDARD_ASPECT).then_some(aspect);
    (scale, shape, snapped.map(|(_, name)| *name))
}

/// How a slot's shape is named as a corner is dragged: as it snapped, or as its width to its height, such as `1.20:1`
/// or `1:1.60`.
fn shape_name(wall: &Wall) -> String {
    let aspect = wall.aspect();
    if let Some((_, name)) = ASPECT_SNAPS
        .iter()
        .find(|(shape, _)| (shape - aspect).abs() < 1e-4)
    {
        return (*name).to_owned();
    }
    if aspect >= 1.0 {
        format!("{aspect:.2}:1")
    } else {
        format!("1:{:.2}", 1.0 / aspect)
    }
}

/// The most `wall` may bend, as the curvature, one over the radius, of its tightest arc: no tighter than
/// [`layout::LEAST_RADIUS`], nor around more than half a circle.
fn most_bend(wall: &Wall) -> f32 {
    let scale = wall.scale();
    let pitch = wall.slot_size().0 + layout::ARC_GAP * scale / layout::ARC_SCALE;
    let around = layout::MOST_ARC / (pitch * wall.grid().columns as f32);
    (1.0 / layout::LEAST_RADIUS).min(around) * 0.999
}

/// Whether `wall` is one the server takes: within its bounds, and for the Tomori wall within its own.
fn allowed(wall: &Wall) -> bool {
    let grid = wall.grid;
    let (width, height) = wall.size();
    grid.is_none_or(|grid| grid.valid())
        && width <= 8.0
        && height <= 4.0
        && (wall.kind != WallKind::Tomori
            || (wall.grid().rows == 1 && wall.grid().columns <= TOMORI_COLUMNS))
        && match wall.shape {
            WallShape::Arc { .. } => wall.radius().is_some(),
            _ => true,
        }
}

/// The radius `wall`, as it was when its knob was taken hold of, bends to when the knob is drawn `travel` metres towards
/// the participant from where it was taken hold of, or flat: its curvature, one over its radius, is as it was and grows
/// evenly with the travel, from flat to [`most_bend`] over [`BEND_TRAVEL`], so that drawing it further bends it more and
/// back towards where it was taken bends it less, never beyond either. Back within [`BEND_REST`] of where it was taken,
/// it is as it was; bending less than [`FLAT_BEND`] of the most it may, unless it was bent less than twice that, it is
/// flat. A radius is a whole number of centimetres, rounded so that the wall stays within half a circle.
pub fn bent(wall: &Wall, travel: f32) -> Option<f32> {
    let start = wall.radius();
    if !travel.is_finite() || travel.abs() < BEND_REST {
        return start;
    }
    let most = most_bend(wall);
    let was = start.map_or(0.0, |radius| 1.0 / radius);
    let curvature = (was + travel * most / BEND_TRAVEL).clamp(0.0, most);
    let flat = if was > 0.0 {
        (FLAT_BEND * most).min(was / 2.0)
    } else {
        FLAT_BEND * most
    };
    if curvature < flat {
        return None;
    }
    let radius = ((1.0 / curvature) * 100.0).ceil() / 100.0;
    (radius <= LONGEST_RADIUS).then_some(radius.max(layout::LEAST_RADIUS))
}

/// The longest radius a wall is curved at, in metres, as the server takes: beyond it, flat.
const LONGEST_RADIUS: f32 = 50.0;

/// The grid whose right edge, or bottom edge, of `wall` stands nearest `at`, in its own frame, along `axis`.
fn nearest_grid(wall: &Wall, axis: Axis, at: Vec3) -> Grid {
    let grid = wall.grid();
    let candidates: Vec<Grid> = match axis {
        Axis::Columns => (1..=layout::MOST_COLUMNS)
            .filter_map(|columns| Grid::new(columns, grid.rows))
            .collect(),
        Axis::Rows => (1..=layout::MOST_ROWS)
            .filter_map(|rows| Grid::new(grid.columns, rows))
            .collect(),
    };
    candidates
        .into_iter()
        .map(|grid| with_grid(wall, grid))
        .filter(allowed)
        .min_by(|a, b| {
            let edge = |wall: &Wall| {
                let [_, top_right, _, bottom_right] = corners(wall);
                match axis {
                    // Measured along the wall, so that a curved wall's edge, coming forward, is where it stands.
                    Axis::Columns => (top_right.x - at.x).abs(),
                    Axis::Rows => (bottom_right.y - at.y).abs(),
                }
            };
            edge(a).total_cmp(&edge(b))
        })
        .map_or(grid, |wall| wall.grid())
}

impl Editor {
    /// The wall taken hold of, by identity.
    pub fn editing(&self) -> Option<&str> {
        self.wall.as_deref()
    }

    /// The wall carried, by identity.
    pub fn carrying(&self) -> Option<&str> {
        self.carry.as_ref().and(self.wall.as_deref())
    }

    /// Let go of the wall taken hold of, as B does when nothing is carried. Returns whether one was.
    pub fn stop(&mut self) -> bool {
        self.drag = None;
        self.carry = None;
        self.preview = None;
        self.confirm = None;
        self.hovered = None;
        self.wall.take().is_some()
    }

    /// B pressed while carrying a wall, with `held` panels on it, at `now`: the wall to remove, for an owner, or `None`
    /// when it holds panels and this is not a second press within [`CONFIRM`], which it then waits for.
    pub fn remove(&mut self, held: usize, now: Instant) -> Option<String> {
        let wall = self.carrying()?.to_owned();
        let confirmed = self
            .confirm
            .is_some_and(|since| now.saturating_duration_since(since) < CONFIRM);
        if held > 0 && !confirmed {
            self.confirm = Some(now);
            return None;
        }
        self.stop();
        self.pending = None;
        Some(wall)
    }

    /// The walls to draw in `projection`: the server's, with the one being changed as it is being changed, or as it was
    /// sent until the server agrees.
    pub fn walls(&self, projection: &Projection) -> Vec<Wall> {
        projection
            .walls
            .iter()
            .map(|wall| {
                let shown = self
                    .preview
                    .as_ref()
                    .or(self.pending.as_ref().map(|(pending, _)| pending))
                    .filter(|shown| shown.id == wall.id);
                shown.cloned().unwrap_or_else(|| wall.clone())
            })
            .collect()
    }

    /// Follow `input`, given `walls` as drawn this frame, and say what to draw and send.
    pub fn update(&mut self, input: Input<'_>, walls: &[(&Wall, Mat4)]) -> Output {
        let mut output = Output::default();
        // The server agreeing, or a while passing, ends what was sent; a wall no longer shown is no longer held.
        if let Some((sent, since)) = &self.pending {
            let agreed = input.projection.wall(&sent.id).is_none_or(|wall| {
                wall.grid() == sent.grid()
                    && (wall.scale() - sent.scale()).abs() < 1e-3
                    && (wall.aspect() - sent.aspect()).abs() < 1e-3
                    && wall.radius().map(|radius| (radius * 100.0).round())
                        == sent.radius().map(|radius| (radius * 100.0).round())
                    && Vec3::from_array(wall.pose.position)
                        .distance(Vec3::from_array(sent.pose.position))
                        < 0.01
            });
            if agreed || input.now.saturating_duration_since(*since) >= PENDING {
                self.pending = None;
            }
        }
        if self
            .wall
            .as_ref()
            .is_some_and(|wall| input.projection.wall(wall).is_none())
            || !input.rights.adjust
        {
            self.stop();
        }
        if self
            .confirm
            .is_some_and(|since| input.now.saturating_duration_since(since) >= CONFIRM)
        {
            self.confirm = None;
        }
        let ray = input.ray.filter(|_| input.active && input.pointing);
        let editing = self
            .wall
            .as_ref()
            .and_then(|id| walls.iter().find(|(wall, _)| wall.id == *id))
            .map(|(wall, frame)| ((*wall).clone(), *frame));
        // The handle the ray meets, unless one is dragged: of those it passes near enough, the one whose middle it passes
        // nearest, as seen from where it begins, so that of two close together the one aimed at is taken.
        let mut met: Option<(Handle, f32)> = None;
        if let (Some((wall, frame)), Some(ray), None, None) =
            (&editing, ray, &self.drag, &self.carry)
        {
            let (points, size) = handle_points(wall, *frame, input.viewer);
            met = nearest_handle(&points, size, &ray);
        }
        let hovered = self
            .drag
            .as_ref()
            .map(|drag| drag.handle)
            .or(met.map(|(handle, _)| handle));
        output.ticked = hovered.is_some() && hovered != self.hovered;
        self.hovered = hovered;
        output.reach = met.map(|(_, along)| along);
        // A squeeze takes the handle the ray meets, or, failing anything nearer, the wall it meets.
        if input.pressed && input.active && input.rights.adjust {
            if let (Some((handle, along)), Some((wall, frame)), Some(ray)) = (met, &editing, ray)
                && along < input.nearer
            {
                output.took = true;
                match handle {
                    Handle::Step(axis, step) => {
                        let grid = wall.grid();
                        let stepped = match axis {
                            Axis::Columns => Grid::new(
                                grid.columns.saturating_add_signed(step.into()),
                                grid.rows,
                            ),
                            Axis::Rows => Grid::new(
                                grid.columns,
                                grid.rows.saturating_add_signed(step.into()),
                            ),
                        };
                        let changed = stepped.map(|grid| with_grid(wall, grid)).filter(allowed);
                        if let Some(changed) = changed {
                            output.change = Some(Change {
                                wall: wall.id.clone(),
                                grid: Some(changed.grid()),
                                scale: None,
                                aspect: None,
                                shape: None,
                                pose: None,
                            });
                            self.pending = Some((changed, input.now));
                        }
                    }
                    handle => {
                        let grab =
                            on_plane(*frame, &ray).map_or(Vec2::ONE, |at| Vec2::new(at.x, at.y));
                        self.drag = Some(Drag {
                            handle,
                            start: wall.clone(),
                            grab,
                            origin: ray.origin,
                            along,
                            point: ray.at(along),
                        });
                    }
                }
            } else if let Some(ray) = ray.filter(|_| self.drag.is_none() && self.carry.is_none()) {
                // The wall's board or an empty slot nearer than anything else: it is taken hold of, and one someone
                // added is carried.
                let taken = walls
                    .iter()
                    .filter_map(|(wall, frame)| {
                        super::wall_hit(wall, *frame, &ray).map(|along| (*wall, *frame, along))
                    })
                    .filter(|(_, _, along)| *along < input.nearer && *along < input.panel)
                    .min_by(|a, b| a.2.total_cmp(&b.2));
                match taken {
                    Some((wall, frame, along)) => {
                        output.took = true;
                        output.ticked = true;
                        self.wall = Some(wall.id.clone());
                        self.confirm = None;
                        if wall.kind == WallKind::Added {
                            let (_, facing, _) = frame.to_scale_rotation_translation();
                            self.carry = Some(Carry {
                                manipulation: Manipulation::new(along),
                                facing,
                                grabbed: frame.inverse().transform_point3(ray.at(along)),
                                model: frame,
                            });
                            self.preview = Some(wall.clone());
                        }
                    }
                    // Squeezed at nothing, it lets go of the wall it held.
                    None => {
                        self.stop();
                    }
                }
            }
        }
        // A handle dragged changes the wall as it would be.
        if let (Some(drag), Some(ray)) = (&self.drag, input.ray) {
            // Where it stood as the handle was taken hold of, which is shown, as one no longer shown is let go of.
            let frame = model(&drag.start.pose).unwrap_or_default();
            let start = &drag.start;
            let changed = match drag.handle {
                // A corner sizes its slots' width and height apart, each as far across as the ray now meets the wall's
                // plane against where it met it as it was taken hold of, the wall growing about its middle.
                Handle::Corner(..) => on_plane(frame, &ray).map(|at| {
                    let (width, height) = start.slot_size();
                    let across = at.x.abs() / drag.grab.x.abs().max(0.05);
                    let down = at.y.abs() / drag.grab.y.abs().max(0.05);
                    let (scale, aspect, _) = slot_shape(width * across, height * down);
                    laid_out(start, start.grid(), scale, aspect, start.radius())
                }),
                Handle::Columns => on_plane(frame, &ray)
                    .map(|at| with_grid(start, nearest_grid(start, Axis::Columns, at.abs()))),
                Handle::Rows => on_plane(frame, &ray).map(|at| {
                    let at = Vec3::new(at.x, -at.y.abs(), at.z);
                    with_grid(start, nearest_grid(start, Axis::Rows, at))
                }),
                // The knob, the last handle dragged: a stepper changes the wall at once, as it is squeezed. It bends as
                // far as the hand has moved towards the participant, along the way the wall faces, level, from where it
                // took hold of it, so that moving up, down or across does nothing; in the desktop window, whose ray
                // begins at the eyes, as far as the point it was taken at has moved up.
                _ => {
                    let travel = match input.controls {
                        Controls::Keyboard => (ray.at(drag.along) - drag.point).y,
                        Controls::Controllers => {
                            let facing = frame.transform_vector3(Vec3::Z);
                            let level = Vec3::new(facing.x, 0.0, facing.z).normalize_or(Vec3::Z);
                            (ray.origin - drag.origin).dot(level)
                        }
                    };
                    Some(laid_out(
                        start,
                        start.grid(),
                        start.scale(),
                        start.aspect,
                        bent(start, travel),
                    ))
                }
            };
            if let Some(changed) = changed.filter(allowed) {
                self.preview = Some(changed);
            }
        }
        // A wall carried follows the ray, facing the viewer, as far along it and as turned as the stick puts it.
        if let (Some(carry), Some(ray)) = (&mut self.carry, input.ray.filter(|_| input.active)) {
            output.ticked |= carry.manipulation.update(input.stick, input.dt);
            let at = ray.at(carry.manipulation.distance());
            // Squared up, it faces the participant, upright, from where it is carried.
            if input.straighten {
                let middle = carry.model.transform_point3(Vec3::ZERO);
                let (_, square, _) = facing(
                    middle,
                    Vec3::new(input.viewer.x, middle.y, input.viewer.z),
                    1.0,
                )
                .to_scale_rotation_translation();
                carry.facing = square;
                carry.manipulation.straighten();
                output.ticked = true;
            }
            carry.model = Mat4::from_translation(at)
                * Mat4::from_rotation_y(carry.manipulation.yaw())
                * Mat4::from_quat(carry.facing)
                * Mat4::from_translation(-carry.grabbed);
            self.preview
                .iter_mut()
                .for_each(|preview| preview.pose = pose(carry.model));
            output.end = Some(at);
            output.hit = Some(crate::pointing::End::held(ray.origin, at));
        }
        // Let go, or taken by something else, what was changed is sent, and shown until the server agrees.
        if input.released || !input.active {
            let before = self
                .wall
                .as_ref()
                .and_then(|id| input.projection.wall(id))
                .cloned();
            let carried = self.carry.take().is_some();
            let dragged = self.drag.take().is_some();
            if let (Some(after), Some(before), true) =
                (self.preview.take(), before, carried || dragged)
            {
                let change = Change {
                    wall: after.id.clone(),
                    grid: (after.grid() != before.grid()).then(|| after.grid()),
                    scale: ((after.scale() - before.scale()).abs() > 1e-4).then(|| after.scale()),
                    aspect: ((after.aspect() - before.aspect()).abs() > 1e-4)
                        .then(|| after.aspect.unwrap_or(0.0)),
                    shape: (after.radius() != before.radius()).then(|| {
                        after
                            .radius()
                            .map_or(WallShape::Grid, |radius| WallShape::Arc { radius })
                    }),
                    pose: (carried
                        && (Vec3::from_array(after.pose.position)
                            .distance(Vec3::from_array(before.pose.position))
                            > 1e-3
                            || Quat::from_array(after.pose.orientation)
                                .angle_between(Quat::from_array(before.pose.orientation))
                                > 1e-3))
                        .then_some(after.pose),
                };
                if change.grid.is_some()
                    || change.scale.is_some()
                    || change.aspect.is_some()
                    || change.shape.is_some()
                    || change.pose.is_some()
                {
                    output.change = Some(change);
                    self.pending = Some((after, input.now));
                }
            }
        }
        // The ray ends on the handle it meets, or the one dragged, where it passes it, as drawn; or else on the wall it
        // meets, where it may take hold of one, and nothing else is taken first.
        let shown = self.walls(input.projection);
        let drawn = self
            .wall
            .as_ref()
            .and_then(|id| shown.iter().find(|wall| wall.id == *id))
            .and_then(|wall| {
                self.carry
                    .as_ref()
                    .map(|carry| carry.model)
                    .or_else(|| model(&wall.pose))
                    .map(|frame| (wall.clone(), frame))
            });
        if self.carry.is_none()
            && let Some(handle) = self.hovered
            && let (Some((wall, frame)), Some(ray)) = (
                &drawn,
                if self.drag.is_some() {
                    input.ray.filter(|_| input.active)
                } else {
                    ray
                },
            )
        {
            let (at, _) = handle_point(wall, *frame, handle, input.viewer);
            let along = (at - ray.origin).dot(ray.direction).max(0.0);
            output.end = Some(ray.at(along));
            output.hit = Some(match self.drag {
                Some(_) => crate::pointing::End::held(ray.origin, ray.at(along)),
                None => crate::pointing::End::new(along, ray.at(along)),
            });
        }
        if output.hit.is_none()
            && input.rights.adjust
            && let Some(ray) = ray
        {
            output.hit = walls
                .iter()
                .filter_map(|(wall, frame)| super::wall_hit(wall, *frame, &ray))
                .min_by(f32::total_cmp)
                .map(|along| crate::pointing::End::new(along, ray.at(along)));
        }
        output.sheets = self.sheets(walls, &input);
        output
    }

    /// The handles of the wall taken hold of, facing the viewer, and the hint below it, as it is drawn.
    fn sheets(&self, walls: &[(&Wall, Mat4)], input: &Input<'_>) -> Vec<Sheet> {
        let Some((wall, frame)) = self
            .wall
            .as_ref()
            .and_then(|id| walls.iter().find(|(wall, _)| wall.id == *id))
            .map(|(wall, frame)| (*wall, *frame))
        else {
            return Vec::new();
        };
        let preview = self
            .preview
            .as_ref()
            .filter(|preview| preview.id == wall.id);
        let wall = preview.unwrap_or(wall);
        let frame = match &self.carry {
            Some(carry) => carry.model,
            None => model(&wall.pose).unwrap_or(frame),
        };
        let mut sheets = Vec::new();
        let (points, size) = handle_points(wall, frame, input.viewer);
        if self.carry.is_none() {
            for (handle, at) in &points {
                sheets.push(Sheet {
                    model: Some(facing(*at, input.viewer, 1.0)),
                    canvas: Some(handle_face(*handle, size, self.hovered == Some(*handle))),
                });
            }
        }
        let held = input
            .projection
            .panels
            .iter()
            .filter(|panel| matches!(&panel.place, tomori_protocol::Place::Wall { wall: on, .. } if *on == wall.id))
            .count();
        // While a handle is dragged, what it makes of the wall, in place of what each control does.
        let hint = match &self.drag {
            Some(drag) => readout(drag.handle, wall),
            None => hint(
                input.controls,
                wall,
                self.carry.is_some(),
                input.rights.remove,
                self.confirm.map(|_| held),
            ),
        };
        // Above the wall and its handles, clear of the status pill, which stands low in the view, facing the viewer at the
        // angle the hint beside a panel carried is drawn.
        let up = frame.transform_vector3(Vec3::Y).normalize_or(Vec3::Y);
        let (knob, _) = handle_point(wall, frame, Handle::Bend, input.viewer);
        let top = points
            .iter()
            .map(|(_, at)| *at)
            .max_by(|a, b| a.dot(up).total_cmp(&b.dot(up)))
            .unwrap_or(knob);
        let above = knob + up * ((top - knob).dot(up) + size);
        sheets.push(hint_above(hint, above, input.viewer));
        sheets
    }
}

/// What a handle dragged makes of `wall`, as it would be let go now: for a corner, its slots' size and shape, such as
/// `60 × 107 cm · 9:16`; for the knob, its curve, such as `◠ radius 2.4 m`, or `flat`; for an edge, its grid, such as
/// `4 × 2 slots`.
pub fn readout(handle: Handle, wall: &Wall) -> Canvas {
    let text = match handle {
        Handle::Corner(..) => {
            let (width, height) = wall.slot_size();
            format!(
                "{:.0} × {:.0} cm · {}",
                width * 100.0,
                height * 100.0,
                shape_name(wall)
            )
        }
        Handle::Bend => wall.radius().map_or("flat".to_owned(), |radius| {
            format!("◠ radius {radius:.1} m")
        }),
        _ => {
            let grid = wall.grid();
            format!("{} × {} slots", grid.columns, grid.rows)
        }
    };
    let columns = crate::ui_text::width(&text) + 2;
    let mut canvas = Canvas::new(columns, HINT_ANGLE);
    canvas.padding = (5, 4);
    canvas.background = [0.0, 0.0, 0.0, 0.82];
    canvas.border = Some(HIGHLIGHT);
    canvas.rows = vec![Row::text(&format!(" {text}"), Style::bold(BRIGHT), columns)];
    canvas
}

/// The hint `canvas` for a wall, standing just above `above`, facing `viewer`, drawn at [`HINT_ANGLE`] a font pixel
/// whatever its distance.
fn hint_above(mut canvas: Canvas, above: Vec3, viewer: Vec3) -> Sheet {
    canvas.pixel = HINT_ANGLE * above.distance(viewer).max(0.1);
    let (_, tall) = canvas.size();
    Sheet {
        model: Some(facing(above + Vec3::Y * tall / 2.0, viewer, 1.0)),
        canvas: Some(canvas),
    }
}

/// The hint beside a wall taken hold of: what each control does to it, as `controls` name them, for `wall`, `carried`
/// or not, saying B removes it where the participant `removes` walls, and, once asked to with panels on it, `confirm`,
/// how many of them would float.
pub fn hint(
    controls: Controls,
    wall: &Wall,
    carried: bool,
    removes: bool,
    confirm: Option<usize>,
) -> Canvas {
    let keyboard = controls == Controls::Keyboard;
    let grip = if keyboard { "G" } else { "Grip" };
    let mut rows: Vec<(String, String, bool)> = Vec::new();
    if carried {
        rows.push((
            (if keyboard { "PgUp PgDn" } else { "Stick ↕" }).to_owned(),
            "further / closer".to_owned(),
            false,
        ));
        rows.push((
            (if keyboard { "[ ]" } else { "Stick ↔" }).to_owned(),
            "turn".to_owned(),
            false,
        ));
        let back = if keyboard { "Backspace" } else { "B" };
        if removes {
            rows.push(match confirm {
                Some(held) => (
                    format!("{back} again"),
                    match held {
                        1 => "remove; 1 panel floats".to_owned(),
                        held => format!("remove; {held} panels float"),
                    },
                    true,
                ),
                None => (back.to_owned(), "remove, for all".to_owned(), true),
            });
        }
        rows.push((
            format!("Let go {}", if keyboard { "G" } else { "" })
                .trim()
                .to_owned(),
            "leave it here".to_owned(),
            false,
        ));
    } else {
        rows.push((
            format!("{grip} ↔ ↕"),
            "more / fewer slots".to_owned(),
            false,
        ));
        if wall.kind != WallKind::Tomori {
            rows.push((format!("{grip} corner"), "size and shape".to_owned(), false));
        }
        rows.push((format!("{grip} - +"), "one fewer / more".to_owned(), false));
        rows.push((
            format!("{grip} ●"),
            (if keyboard {
                "drag up to bend"
            } else {
                "pull to bend"
            })
            .to_owned(),
            false,
        ));
        if wall.kind == WallKind::Added {
            rows.push((format!("{grip} wall"), "carry it".to_owned(), false));
        }
        rows.push((
            (if keyboard { "Backspace" } else { "B" }).to_owned(),
            "done".to_owned(),
            false,
        ));
    }
    let keys = rows
        .iter()
        .map(|(control, _, _)| crate::ui_text::width(control))
        .max()
        .unwrap_or_default()
        + 2;
    let columns = keys + 22;
    let mut canvas = Canvas::new(columns, HINT_ANGLE);
    canvas.padding = (5, 4);
    canvas.background = [0.0, 0.0, 0.0, 0.82];
    canvas.border = Some(FRAME);
    canvas.rows = rows
        .iter()
        .map(|(control, action, warm)| {
            let (key, text) = if *warm {
                (Style::bold(THROW), Style::plain(THROW))
            } else {
                (Style::bold(BRIGHT), Style::plain(TEXT))
            };
            Row::new(cut(
                &[
                    crate::ui_text::Span::new(crate::ui_text::pad(control, keys), key),
                    crate::ui_text::Span::new(action.as_str(), text),
                ],
                columns,
            ))
        })
        .collect();
    canvas
}

#[cfg(test)]
mod tests;
