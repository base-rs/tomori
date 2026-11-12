// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Where a wall's handles, its padlock and its bar stand: each at a fixed place in the wall's own frame, sized by the
//! wall's scale, laid out from the wall and its group alone, and which of them a ray meets. See `design/projection.md`.
use glam::{Mat4, Vec3};
use tomori_protocol::{Edge, Wall, WallKind, group};

use super::{
    Axis, HANDLE_REACH, HANDLE_SIZE, HANDLE_SIZES, Handle, LockControls, OUTSET, PADLOCK_SPACING,
    SPACING, model,
};
use crate::pointing::Ray;

/// A place a locked wall's padlock may stand: its face's model, unscaled, and whether it is below the wall, so that
/// what it says stands below it, clear of the wall.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct PadlockPlace {
    pub(super) model: Mat4,
    pub(super) below: bool,
    /// Where along its edge it stands, or `None` for where [`handle_points_with`] puts it.
    pub(super) spot: Option<LockSpot>,
}

/// Where along a wall's top or bottom edge its padlock stands, from beyond its left end (0) to beyond its right (1), so
/// that the padlock of a wall unlocked there stays there as the wall is taken hold of and changed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct LockSpot {
    top: bool,
    along: f32,
}

impl LockSpot {
    /// Where it stands on `wall`'s surface, standing at `frame`, from its pose, among what is `around` it, with its
    /// handles `size` across: on the row above its top or below its bottom, or `None` where another wall of its group
    /// is joined across that edge, where it would stand over another wall's slots or by its group's bar, or within
    /// [`FLOOR_CLEARANCE`] of the floor.
    fn place(self, wall: &Wall, frame: Mat4, around: &Around, size: f32) -> Option<(f32, f32)> {
        let Edges {
            pitch,
            side,
            bottom,
            lock,
            ..
        } = Edges::of(wall, frame);
        let joined = |edge: Edge| around.joined.contains(&edge);
        let half = wall.size().0 / 2.0;
        let left = if joined(Edge::Left) { -half } else { -side };
        let right = if joined(Edge::Right) { half } else { side };
        let (edge, y) = if self.top {
            (Edge::Top, lock)
        } else {
            (Edge::Bottom, bottom - pitch)
        };
        let x = left + (right - left) * self.along;
        let at = along(wall, x).0 + Vec3::Y * y;
        let above = frame.transform_point3(at).y - size / 2.0
            >= tomori_protocol::cluster::FLOOR + FLOOR_CLEARANCE;
        (!joined(edge) && above && around.clear(x, y, size)).then_some((x, y))
    }
}

/// Which handles `wall` has, for a participant who may adjust it, with the lock's `controls`: the Tomori wall's grid
/// only in its one row, and its slots their own size; and a wall locked, where the server locks walls, its padlock
/// alone, so neither the hinge.
pub(super) fn handles(wall: &Wall, controls: LockControls) -> Vec<Handle> {
    let mut lock = Vec::new();
    if controls.padlock {
        lock.push(Handle::Lock);
        if wall.is_locked() {
            return lock;
        }
    }
    if controls.reset {
        lock.push(Handle::Reset);
    }
    // A wall in a group stands upright, so that the group reads as one board, and has no hinge.
    let mut handles = vec![
        Handle::Columns,
        Handle::Step(Axis::Columns, -1),
        Handle::Step(Axis::Columns, 1),
        Handle::Bend,
        Handle::Text,
    ];
    if wall.group.is_none() {
        handles.push(Handle::Tilt);
    }
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
    handles.extend(lock);
    handles
}

/// How large `wall`'s handles are, in metres: [`HANDLE_SIZE`] at its scale, within [`HANDLE_SIZES`]. It follows from
/// the wall alone, never from where the participant is, so that its handles stand where they do however they move.
pub fn handle_size(wall: &Wall) -> f32 {
    (HANDLE_SIZE * wall.scale()).clamp(HANDLE_SIZES.0, HANDLE_SIZES.1)
}

/// Where each of `wall`'s handles stands, the wall standing at `frame`, in the space, and how large a handle is, the same
/// for every one, [`handle_size`]. Each stands at a fixed place in the wall's own frame, whatever the participant's
/// place or gaze, on a frame [`OUTSET`] handles beyond its edges: its corners diagonally beyond its corners; the knob
/// above the middle of its top edge; the handle of its columns beside the middle of its right edge, with the steppers of
/// its columns beyond it, fewer then more; the hinge beside the middle of its left edge, on the line it tilts about; its
/// [`Bar`] centred below its bottom edge, level with its bottom corners, and on its row the text handle to its left and
/// the handle of its rows between their steppers, fewer first, to its right, beside it where they fit within its bottom
/// corners and otherwise beyond them. A small wall's frame is as large as its handles need, so that every handle's
/// middle stands at least [`SPACING`] handles from every other's, and its face, in the wall's plane, clear of every
/// other's and of the bar's, from wherever it is seen. None stands within [`FLOOR_CLEARANCE`] of the floor: below a wall
/// standing low, the bar and its row stand a row above its top, and a handle still that low is left out.
pub fn handle_points(wall: &Wall, frame: Mat4) -> (Vec<(Handle, Vec3)>, f32) {
    handle_points_among(wall, frame, &[], None)
}

/// Where each of `wall`'s handles stands, as [`handle_points`] lays them out, with the lock's `controls`: the padlock a
/// row beyond its top left corner and the control that puts it back beside it; or, where it is locked, the padlock
/// alone, where it would stand.
pub fn handle_points_with(
    wall: &Wall,
    frame: Mat4,
    controls: LockControls,
) -> (Vec<(Handle, Vec3)>, f32) {
    handle_points_among_with(wall, frame, &[], None, controls)
}

/// Where `wall`'s handles stand, as [`handle_points`] lays them out, among `walls` as they are drawn, where it is one of
/// a group: those along an edge another wall of its group is joined across stand along the opposite edge instead, and
/// any that would still stand over the slots of another wall of its group, or by its group's bar, are left out, but
/// for `keep`, the handle being dragged. So none covers what the group shows.
pub fn handle_points_among(
    wall: &Wall,
    frame: Mat4,
    walls: &[&Wall],
    keep: Option<Handle>,
) -> (Vec<(Handle, Vec3)>, f32) {
    handle_points_among_with(wall, frame, walls, keep, LockControls::default())
}

/// Where `wall`'s handles stand among `walls`, as [`handle_points_among`] lays them out, with the lock's `controls`, as
/// [`handle_points_with`] places them: the padlock, which says why a wall will not change, is never left out, and
/// stands below where another wall is joined across its top, and above its top right corner where one is joined across
/// its left edge.
pub fn handle_points_among_with(
    wall: &Wall,
    frame: Mat4,
    walls: &[&Wall],
    keep: Option<Handle>,
    controls: LockControls,
) -> (Vec<(Handle, Vec3)>, f32) {
    let (models, size) = handle_models(wall, frame, walls, keep, controls);
    let points = models
        .into_iter()
        .map(|(handle, model)| (handle, model.w_axis.truncate()))
        .collect();
    (points, size)
}

/// Where each of `wall`'s handles stands among `walls`, as [`handle_points_among_with`] lays them out, as its face's
/// model, unscaled: in the wall's plane where it stands, facing as the wall faces there.
pub(super) fn handle_models(
    wall: &Wall,
    frame: Mat4,
    walls: &[&Wall],
    keep: Option<Handle>,
    controls: LockControls,
) -> (Vec<(Handle, Mat4)>, f32) {
    handle_models_at(wall, frame, walls, keep, controls, None)
}

/// Where each of `wall`'s handles stands, as [`handle_models`] lays them out, but for its padlock, at `lock` along its
/// edge, where it stood as it was unlocked, and the control that puts it back beside it, where it stands there.
pub(super) fn handle_models_at(
    wall: &Wall,
    frame: Mat4,
    walls: &[&Wall],
    keep: Option<Handle>,
    controls: LockControls,
    lock: Option<LockSpot>,
) -> (Vec<(Handle, Mat4)>, f32) {
    let mut around = Around::of(wall, frame, walls, keep);
    around.controls = controls;
    around.lock = lock;
    let (spots, size) = lay_out(wall, frame, &around);
    let models = spots
        .into_iter()
        .map(|(handle, spot)| (handle, spot.model(frame)))
        .collect();
    (models, size)
}

/// Where a locked wall's padlock may stand, `wall` standing at `frame` among `walls` as they are drawn, as a face's
/// model, unscaled, and how large it is: first where [`handle_points_with`] puts it, a row beyond its top left corner;
/// then along the row above its top edge and the row below its bottom edge, from beyond one corner to beyond the
/// other, at most [`PADLOCK_SPACING`] apart, as a curved wall runs. In a group, none stands along an edge another
/// wall is joined across, beyond its own slots where one is joined beside it, or over another wall's slots or by its
/// group's bar; and none within [`FLOOR_CLEARANCE`] of the floor. The owner sees it at whichever is nearest them.
pub(super) fn padlock_places(
    wall: &Wall,
    frame: Mat4,
    walls: &[&Wall],
) -> (Vec<PadlockPlace>, f32) {
    let controls = LockControls {
        padlock: true,
        reset: false,
    };
    let mut around = Around::of(wall, frame, walls, None);
    around.controls = controls;
    let (laid, size) = lay_out(wall, frame, &around);
    let mut spots: Vec<(Spot, Option<LockSpot>)> = laid
        .into_iter()
        .filter(|(handle, _)| *handle == Handle::Lock)
        .map(|(_, spot)| (spot, None))
        .collect();
    let side = Edges::of(wall, frame).side;
    // As many as keep them at most `PADLOCK_SPACING` apart from beyond one end to beyond the other.
    let across = if around.joined.contains(&Edge::Left) {
        wall.size().0 / 2.0
    } else {
        side
    } + if around.joined.contains(&Edge::Right) {
        wall.size().0 / 2.0
    } else {
        side
    };
    let steps = (across / PADLOCK_SPACING).ceil().max(1.0) as u32;
    for top in [true, false] {
        for step in 0..=steps {
            let lock = LockSpot {
                top,
                along: step as f32 / steps as f32,
            };
            let Some((x, y)) = lock.place(wall, frame, &around, size) else {
                continue;
            };
            let (at, runs) = along(wall, x);
            let spot = Spot {
                at: at + Vec3::Y * y,
                runs,
            };
            if spots
                .iter()
                .all(|(other, _)| other.at.distance(spot.at) > size)
            {
                spots.push((spot, Some(lock)));
            }
        }
    }
    let places = spots
        .into_iter()
        .map(|(spot, lock)| PadlockPlace {
            model: spot.model(frame),
            below: spot.at.y < 0.0,
            spot: lock,
        })
        .collect();
    (places, size)
}

/// Where a locked wall's padlock may stand, `wall` standing at `frame` among `walls` as they are drawn, and how large it
/// is: the first where [`handle_points_with`] puts it, and the rest along its top and bottom edges. See
/// `design/projection.md`.
pub fn padlock_points(wall: &Wall, frame: Mat4, walls: &[&Wall]) -> (Vec<Vec3>, f32) {
    let (places, size) = padlock_places(wall, frame, walls);
    let points = places
        .into_iter()
        .map(|place| place.model.w_axis.truncate())
        .collect();
    (points, size)
}

/// Where an owner at `viewer` first sees a locked wall's padlock, `wall` standing at `frame` among `walls`: the place of
/// [`padlock_points`] nearest them, the first of those as near, or `None` where it stands nowhere.
pub fn padlock_point(wall: &Wall, frame: Mat4, walls: &[&Wall], viewer: Vec3) -> Option<Vec3> {
    let (points, _) = padlock_points(wall, frame, walls);
    points
        .get(nearest_place(points.iter().copied(), viewer))
        .copied()
}

/// Which of `points` is nearest `viewer`, the first of those as near.
pub(super) fn nearest_place(points: impl Iterator<Item = Vec3>, viewer: Vec3) -> usize {
    points
        .map(|point| point.distance(viewer))
        .enumerate()
        .fold((0, f32::INFINITY), |best, (index, distance)| {
            if distance < best.1 {
                (index, distance)
            } else {
                best
            }
        })
        .0
}

/// Where `wall`'s grab bar stands, the wall standing at `frame`, as [`handle_points`] lays it out with its handles.
pub fn bar(wall: &Wall, frame: Mat4) -> Bar {
    placed_bar(wall, frame, &[]).at(wall, frame)
}

/// Where `wall`'s grab bar stands among `walls` as they are drawn, as [`bar`] says, if it shows one: a group shows one
/// bar, below its bottom left wall, whose slots reach lowest, centred below the slots of the whole group, which carries
/// the whole group as any of its walls does. A locked wall shows none, as it is not taken hold of. See
/// `design/projection.md`.
pub fn group_bar(wall: &Wall, frame: Mat4, walls: &[&Wall]) -> Option<Bar> {
    (!wall.is_locked() && crate::projection::boards::bears_bar(wall, walls)).then(|| {
        placed_bar(
            wall,
            frame,
            &crate::projection::boards::members_slots(wall, walls),
        )
        .at(wall, frame)
    })
}

/// What stands about a wall of a group, which its handles keep clear of: the slots of the other walls of its group, as
/// they stand on its surface, from its pose; the edges another wall of its group is joined across; its group's bar, as
/// it stands on its surface; and the handle being dragged, which stays where it is.
#[derive(Clone, Debug, Default)]
struct Around {
    slots: Vec<group::Extent>,
    joined: Vec<Edge>,
    keep: Option<Handle>,
    /// Its group's bar, on its surface, where it is one of a group.
    bar: Option<PlacedBar>,
    /// The lock's controls it shows.
    controls: LockControls,
    /// Where along its edge its padlock stands, where it was unlocked there.
    lock: Option<LockSpot>,
}

impl Around {
    fn of(wall: &Wall, frame: Mat4, walls: &[&Wall], keep: Option<Handle>) -> Self {
        let slots = crate::projection::boards::members_slots(wall, walls);
        let bar = if slots.is_empty() {
            None
        } else if crate::projection::boards::bears_bar(wall, walls) {
            Some(placed_bar(wall, frame, &slots))
        } else {
            crate::projection::boards::members(wall, walls)
                .into_iter()
                .find(|other| crate::projection::boards::bears_bar(other, walls))
                .map(|bearer| {
                    // Standing as it stands from this wall.
                    let at = frame
                        * model(&wall.pose).unwrap_or_default().inverse()
                        * model(&bearer.pose).unwrap_or_default();
                    let placed = placed_bar(
                        bearer,
                        at,
                        &crate::projection::boards::members_slots(bearer, walls),
                    );
                    let (u, y, _, _) = group::on_surface(&wall.pose, wall.radius(), &bearer.pose);
                    PlacedBar {
                        centre: placed.centre + u,
                        row: placed.row + y,
                        ..placed
                    }
                })
        };
        Self {
            slots,
            joined: crate::projection::boards::joined_edges(wall, walls),
            keep,
            bar,
            controls: LockControls::default(),
            lock: None,
        }
    }

    /// Whether a handle `size` across, its middle `x` and `y` from a wall's pose on its surface, stands over none of the
    /// slots about it, and so far from its group's bar that the ray never meets both.
    fn clear(&self, x: f32, y: f32, size: f32) -> bool {
        let half = size / 2.0;
        let slots = self.slots.iter().all(|slots| {
            x + half <= slots.left
                || x - half >= slots.right
                || y + half <= slots.bottom
                || y - half >= slots.top
        });
        slots
            && self.bar.is_none_or(|bar| {
                let reach = HANDLE_REACH * (size + bar.size);
                (y - bar.row).abs() >= reach || (x - bar.centre).abs() >= bar.half + reach
            })
    }
}

/// A wall's grab bar: a thin bar a participant who may adjust walls takes hold of the wall by with the grip, as they do
/// its board, so that a wall whose every slot holds a panel, and which is curved, is as easy to take hold of as any.
/// It stands centred below the wall's bottom edge, or a curved wall's label, facing the way the wall faces there, about
/// a third of the wall long. See `design/projection.md`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bar {
    /// Its middle, in the space.
    pub middle: Vec3,
    /// From its middle to its right end, in the space.
    pub half: Vec3,
    /// The size of a handle of its wall, which its thickness and how near the ray passes it to meet it are measured
    /// in.
    pub size: f32,
    /// Where it stands, facing the way its wall faces at its middle.
    pub model: Mat4,
}

impl Bar {
    /// How far along `ray` it is met, if the ray crosses its plane, its wall's, in front of where it begins, within
    /// [`HANDLE_REACH`] of a handle's size of its middle line, up or down, and of its ends, as near as a handle is met,
    /// so that it is as easy to hold, and its reach and every handle's leave a gap between them in the wall's plane.
    pub fn hit(&self, ray: &Ray) -> Option<f32> {
        let (along, across, up) = on_face(self.model, ray)?;
        let reach = self.size * HANDLE_REACH;
        (across.abs() <= self.half.length() + reach && up.abs() <= reach).then_some(along)
    }
}

/// Where `ray` crosses the plane of a face standing at `model`, in front of where it begins: how far along the ray, and
/// how far from the face's middle, across it and up it.
fn on_face(model: Mat4, ray: &Ray) -> Option<(f32, f32, f32)> {
    let (across, up) = (
        model.x_axis.truncate().normalize_or(Vec3::X),
        model.y_axis.truncate().normalize_or(Vec3::Y),
    );
    let normal = across.cross(up);
    let towards = normal.dot(ray.direction);
    let middle = model.w_axis.truncate();
    let along = (middle - ray.origin).dot(normal) / towards;
    // A ray along the plane, or away from it, crosses it nowhere in front of it.
    if towards.abs() < 1e-6 || along <= 0.0 {
        return None;
    }
    let off = ray.at(along) - middle;
    Some((along, off.dot(across), off.dot(up)))
}

/// How thick a bar is drawn, in handles: thin, though it is met as near as a handle is.
pub(super) const BAR_THICKNESS: f32 = 0.45;

/// How long a bar is, in handles, at least and at most, about a third of the slots it stands below between.
pub(super) const BAR_LEAST: f32 = 3.0;
pub(super) const BAR_MOST: f32 = 5.0;

/// Half the length of a bar below slots `across` wide, of a wall whose handles are `size`: a third of them, within
/// [`BAR_LEAST`] and [`BAR_MOST`] handles.
fn bar_half(across: f32, size: f32) -> f32 {
    (across / 6.0).clamp(BAR_LEAST * size / 2.0, BAR_MOST * size / 2.0)
}

/// How far above the space's floor everything a wall shows stands, at least, in metres: its bar and its handles, so that
/// none is lost in the floor nor stands where only a crouch reaches it.
pub const FLOOR_CLEARANCE: f32 = 0.15;

/// Where a wall's handles stand from, on its surface, from its pose: the size of a handle and the spacing of their
/// middles; how far to either side the columns of handles beside its edges stand; how far up and down the rows of
/// handles above and below it; the row its bar stands on, with the handle of its rows, their steppers and the text
/// handle beside it, and half the bar's length. They stand [`OUTSET`] handles beyond what it draws, a curved wall's
/// label below its slots included, but never so near its middle that its handles and its bar come nearer each other
/// than [`SPACING`] handles: a small wall's are as far out as they need. The bar's row is its bottom row, level with
/// its bottom corners, unless that would stand within [`FLOOR_CLEARANCE`] of the floor, standing at `frame`, when it is
/// a row above its top, and the padlock's a row above that.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Edges {
    size: f32,
    pitch: f32,
    side: f32,
    top: f32,
    bottom: f32,
    row: f32,
    half: f32,
    /// The row the padlock and the control that puts the wall back stand on.
    lock: f32,
}

impl Edges {
    fn of(wall: &Wall, frame: Mat4) -> Self {
        let size = handle_size(wall);
        let (pitch, out) = (SPACING * size, OUTSET * size);
        let (width, height) = wall.size();
        let label = match wall.radius() {
            Some(_) => {
                crate::projection::ARC_LABEL_DROP
                    + crate::projection::arc_label(wall).size().1 / 2.0
            }
            None => 0.0,
        };
        let half = bar_half(width, size);
        let top = (height / 2.0 + out).max(pitch);
        let bottom = -(height / 2.0 + label + out).max(pitch);
        let low = frame
            .transform_point3(along(wall, 0.0).0 + Vec3::Y * bottom)
            .y
            - size / 2.0;
        let raised = low < tomori_protocol::cluster::FLOOR + FLOOR_CLEARANCE;
        Self {
            size,
            pitch,
            side: (width / 2.0 + out).max(half + pitch).max(2.0 * pitch),
            top,
            bottom,
            row: if raised { top + pitch } else { bottom },
            half,
            lock: if raised {
                top + 2.0 * pitch
            } else {
                top + pitch
            },
        }
    }
}

/// Where a wall's bar stands on its surface, from its pose: how far along it its middle is, how far up, half its length
/// and the size of a handle of its wall.
#[derive(Clone, Copy, Debug, PartialEq)]
struct PlacedBar {
    centre: f32,
    row: f32,
    half: f32,
    size: f32,
}

impl PlacedBar {
    /// It, on `wall` standing at `frame`, in the space: below the wall as its handles stand along it, or, centred below
    /// the slots of its group beyond the wall's own, along a curved wall's arc, turned as the arc runs there.
    fn at(self, wall: &Wall, frame: Mat4) -> Bar {
        let spot = match wall.radius() {
            // Along the wall as its handles are, where it is below it.
            _ if self.centre.abs() <= wall.size().0 / 2.0 => {
                let (at, runs) = along(wall, self.centre);
                Spot {
                    at: at + Vec3::Y * self.row,
                    runs,
                }
            }
            None => Spot {
                at: Vec3::new(self.centre, self.row, 0.0),
                runs: Vec3::X,
            },
            Some(radius) => {
                let (sin, cos) = (self.centre / radius).sin_cos();
                Spot {
                    at: Vec3::new(radius * sin, self.row, radius * (1.0 - cos)),
                    runs: Vec3::new(cos, 0.0, sin),
                }
            }
        };
        let model = spot.model(frame);
        Bar {
            middle: model.w_axis.truncate(),
            half: model.x_axis.truncate() * self.half,
            size: self.size,
            model,
        }
    }
}

/// Where `wall`'s bar stands on its surface, below its bottom row, centred below its slots and `slots`, those of the
/// other walls of its group it bears the bar of, as they stand on its surface, a third of them long.
fn placed_bar(wall: &Wall, frame: Mat4, slots: &[group::Extent]) -> PlacedBar {
    let edges = Edges::of(wall, frame);
    if slots.is_empty() {
        return PlacedBar {
            centre: 0.0,
            row: edges.row,
            half: edges.half,
            size: edges.size,
        };
    }
    let own = wall.extent();
    let (left, right, highest, lowest) = slots.iter().fold(
        (own.left, own.right, own.top, own.bottom),
        |(left, right, top, bottom), slots| {
            (
                left.min(slots.left),
                right.max(slots.right),
                top.max(slots.top),
                bottom.min(slots.bottom),
            )
        },
    );
    // Raised clear of the floor, above the highest slots of the group, as far beyond them as above its own; or below the
    // lowest, as far below them as below its own, where a locked wall's reach lower than its own by a centimetre.
    let row = if edges.row > edges.top {
        highest - own.top + edges.row
    } else if own.bottom - lowest > 0.01 {
        lowest - own.bottom + edges.row
    } else {
        edges.row
    };
    PlacedBar {
        centre: (left + right) / 2.0,
        row,
        half: bar_half(right - left, edges.size),
        size: edges.size,
    }
}

/// Where a handle or a bar stands in its wall's own frame: its middle, and the way the wall runs there, which it is
/// turned along, facing as the wall faces there.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Spot {
    at: Vec3,
    runs: Vec3,
}

impl Spot {
    /// Where it stands in the space, its wall standing at `frame`, unscaled.
    pub(super) fn model(self, frame: Mat4) -> Mat4 {
        frame
            * Mat4::from_cols(
                self.runs.extend(0.0),
                Vec3::Y.extend(0.0),
                self.runs.cross(Vec3::Y).extend(0.0),
                self.at.extend(1.0),
            )
    }
}

/// Where `x` along `wall` stands, level with its middle, in its own frame, and which way the wall runs there: a flat
/// wall's board, or a curved wall's slots, each flat, as they are drawn, its middle on the arc and turned to face the
/// line it curves about, so that a point along the wall is on the slot whose column it is in, or across the gap between
/// two, and beyond its sides, along the slot at that end, as the wall would go on. Measured along the arc alone, a wall
/// of few wide slots, such as one slot five times a standard one, curved tightly, would have its handles beside its
/// edges far in front of where its slots end.
pub(super) fn along(wall: &Wall, x: f32) -> (Vec3, Vec3) {
    let Some(radius) = wall.radius() else {
        return (Vec3::new(x, 0.0, 0.0), Vec3::X);
    };
    let right = wall.size().0 / 2.0;
    let columns = wall.grid().columns as f32;
    let (slot, gap) = (wall.slot_size().0, wall.gap());
    let step = slot + gap;
    // `from` the middle of `column`'s slot, along it.
    let on_slot = |column: f32, from: f32| {
        let angle = (column - (columns - 1.0) / 2.0) * step / radius;
        let (sin, cos) = angle.sin_cos();
        let runs = Vec3::new(cos, 0.0, sin);
        (
            Vec3::new(radius * sin, 0.0, radius * (1.0 - cos)) + runs * from,
            runs,
        )
    };
    let column = (x / step + (columns - 1.0) / 2.0)
        .round()
        .clamp(0.0, columns - 1.0);
    let from = x - (column - (columns - 1.0) / 2.0) * step;
    if from.abs() <= slot / 2.0 || x.abs() >= right {
        return on_slot(column, from);
    }
    // Across the gap beside its slot, from its edge to its neighbour's.
    let side = from.signum();
    let (edge, _) = on_slot(column, side * slot / 2.0);
    let (other, _) = on_slot(column + side, -side * slot / 2.0);
    let across = (from.abs() - slot / 2.0) / gap;
    (
        edge.lerp(other, across),
        (other - edge).normalize_or(Vec3::X) * side,
    )
}

/// Where each of `wall`'s handles that `around` leaves stands in its own frame, as [`handle_points`] lays them out, and
/// how large they are: a function of the wall and its group alone, each handle placed once, with no search.
fn lay_out(wall: &Wall, frame: Mat4, around: &Around) -> (Vec<(Handle, Spot)>, f32) {
    let Edges {
        size,
        pitch,
        side,
        top,
        bottom,
        row,
        half,
        lock,
    } = Edges::of(wall, frame);
    let moved = around
        .lock
        .and_then(|lock| lock.place(wall, frame, around, size));
    let joined = |edge: Edge| around.joined.contains(&edge);
    let (left, right, up, down) = (
        joined(Edge::Left),
        joined(Edge::Right),
        joined(Edge::Top),
        joined(Edge::Bottom),
    );
    // Where each stands along the wall and up it. In a group, those along an edge another wall is joined across stand
    // along the opposite edge, where none is, and nowhere otherwise; a corner beside a joined edge is left out. A wall
    // in a group has no hinge, so the handles of its right edge stand along its left in its place.
    let place = |handle: Handle| -> Option<(f32, f32)> {
        Some(match handle {
            Handle::Corner(across, upward) => {
                let beside = if across < 0 { left } else { right };
                let level = if upward > 0 { up } else { down };
                if beside || level {
                    return None;
                }
                (
                    f32::from(across) * side,
                    if upward > 0 { top } else { bottom },
                )
            }
            Handle::Bend => match (up, down) {
                (false, _) => (0.0, top),
                (true, false) => (0.0, bottom - pitch),
                _ => return None,
            },
            Handle::Columns | Handle::Step(Axis::Columns, _) => {
                let beyond = match handle {
                    Handle::Step(_, step) if step < 0 => 1.0,
                    Handle::Step(..) => 2.0,
                    _ => 0.0,
                };
                let towards = match (right, left) {
                    (false, _) => 1.0,
                    (true, false) => -1.0,
                    _ => return None,
                };
                (towards * (side + beyond * pitch), 0.0)
            }
            // On the bar's row: the text handle to its left, the handle of its rows between their steppers to its right,
            // beside it where they fit within its bottom corners, and otherwise beyond them.
            Handle::Rows | Handle::Step(Axis::Rows, _) | Handle::Text => {
                let from = if side >= half + 4.0 * pitch {
                    half
                } else {
                    side
                };
                let x = match handle {
                    Handle::Text => -(from + pitch),
                    Handle::Step(_, step) => from + (2.0 + f32::from(step)) * pitch,
                    _ => from + 2.0 * pitch,
                };
                let row = match (down, up) {
                    (false, _) => row,
                    (true, false) => lock + pitch,
                    _ => return None,
                };
                (x, row)
            }
            Handle::Tilt => (-side, 0.0),
            Handle::Lock | Handle::Reset => {
                let towards = if left && !right { -1.0 } else { 1.0 };
                let inward = if handle == Handle::Reset { pitch } else { 0.0 };
                let row = if up && !down { bottom - pitch } else { lock };
                let home = (towards * (inward - side), row);
                // Where it was unlocked along its edge, the control that puts it back beside it, inwards, and left
                // out, as any handle is, where that is not clear.
                match (moved, handle) {
                    (Some(at), Handle::Lock) => at,
                    (Some((x, y)), _) => (if x > 0.0 { x - pitch } else { x + pitch }, y),
                    _ => home,
                }
            }
        })
    };
    // None stands over another wall's slots or by its group's bar, but the handle being dragged and the padlock, which
    // says why a wall will not change; and none within [`FLOOR_CLEARANCE`] of the floor, but the handle being dragged.
    let spots = handles(wall, around.controls)
        .into_iter()
        .filter_map(|handle| {
            let (x, y) = place(handle)?;
            let (at, runs) = along(wall, x);
            let spot = Spot {
                at: at + Vec3::Y * y,
                runs,
            };
            let dragged = around.keep == Some(handle);
            let clear = handle == Handle::Lock || around.clear(x, y, size);
            let above = frame.transform_point3(spot.at).y - size / 2.0
                >= tomori_protocol::cluster::FLOOR + FLOOR_CLEARANCE;
            (dragged || (clear && above)).then_some((handle, spot))
        })
        .collect();
    (spots, size)
}

/// The handle of `models`, each `size` across, that `ray` meets, and how far along it: where it crosses a handle's plane
/// in front of it, within [`HANDLE_REACH`] of a handle's size of its middle, across or up its face, the nearest along
/// it. Neighbours' reaches leave a gap between them in their wall's plane, so that the ray meets at most one of two.
pub(super) fn nearest_handle(
    models: &[(Handle, Mat4)],
    size: f32,
    ray: &Ray,
) -> Option<(Handle, f32)> {
    let reach = size * HANDLE_REACH;
    models
        .iter()
        .filter_map(|(handle, model)| {
            let (along, across, up) = on_face(*model, ray)?;
            (across.abs() <= reach && up.abs() <= reach).then_some((*handle, along))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
}

/// Where `handle` of a wall like `wall`, standing at `frame`, is drawn, and how large, as [`handle_points`] lays them
/// out, or the wall's middle for a handle it does not have.
pub fn handle_point(wall: &Wall, frame: Mat4, handle: Handle) -> (Vec3, f32) {
    handle_point_among(wall, frame, handle, &[], None)
}

/// Where `handle` of `wall` is drawn among `walls`, as [`handle_points_among`] lays them out, keeping `keep`, and how
/// large, or the wall's middle for a handle it does not show.
pub fn handle_point_among(
    wall: &Wall,
    frame: Mat4,
    handle: Handle,
    walls: &[&Wall],
    keep: Option<Handle>,
) -> (Vec3, f32) {
    let (points, size) = handle_points_among(wall, frame, walls, keep);
    let at = points
        .into_iter()
        .find(|(shown, _)| *shown == handle)
        .map_or(frame.transform_point3(Vec3::ZERO), |(_, at)| at);
    (at, size)
}
