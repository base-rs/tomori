// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Editing walls with the grip: taking hold of a wall by pointing at its board or an empty slot, the handles that then
//! appear on it, small and grabbed with the grip, to size its slots, give it columns and rows, step them one at a time,
//! bend it and tilt it, carrying a wall someone added as a panel is carried, and removing one, as an owner, with B. A wall
//! carried carries its group, and snaps beside another it is brought within reach of, its place outlined; pulled firmly
//! towards the participant, it comes out of its group alone. Each change is shown at once, as the wall would be, and sent
//! to the server when it is let go, shown so until the server's projection agrees or [`super::PENDING`] passes. See
//! `design/projection.md`.
use std::time::{Duration, Instant};

use glam::{Mat4, Quat, Vec2, Vec3};
use tomori_protocol::{
    Attach, Edge, Grid, Pose, Projection, Wall, WallKind, WallShape, group, wall as layout,
};

use super::{
    Cells, Controls, FRAME, HIGHLIGHT, HINT_ANGLE, LIFT, PENDING, THROW, ghost, slot_unit,
};
use crate::{
    board::{model, pose},
    canvas::Canvas,
    controller_ui::Sheet,
    manipulation::Manipulation,
    pointing::Ray,
    recording::facing,
    transcript::{BRIGHT, DIM, TEXT},
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

/// How far a corner dragged sizes a wall's slots, as the power of how far from the wall's middle it is against where it
/// was taken hold of: twice as far makes them four times as large, so that from a standard slot to the largest is one
/// drag, to a little over twice as far.
pub const CORNER_GAIN: f32 = 2.0;

/// How far the hand moves, in metres, past [`DEAD_ZONE`] from where it took hold of the knob or the hinge, to take a
/// wall through the whole range: the knob across, to the right, from flat to as tightly as it may curve, and the hinge
/// up or down from one end of [`layout::TILTS`] to the other, half of it from upright to either end. It is the same
/// for a wall of any size, a comfortable movement of the hand alone, so that a large wall needs no reach and a small one
/// no more care; [`ease`] keeps a little fine to set.
pub const TRAVEL: f32 = 0.25;

/// How far the hand moves, in metres, from where it took hold of the knob across or of the hinge up or down, before it
/// bends or tilts the wall at all: a hand held still drifts and shakes by less, so that taking hold changes nothing, and
/// moving back to where it took hold is the wall as it was.
pub const DEAD_ZONE: f32 = 0.01;

/// How little a wall bends, as a proportion of the most it may, before it snaps flat.
const FLAT_BEND: f32 = 0.06;

/// How near upright a wall tilts, in degrees, before it snaps upright.
pub const UPRIGHT_SNAP: f32 = 3.0;

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

/// How far a wall carried in a group is pulled towards the participant, from where it was taken hold of, to come out of
/// its group alone, in metres: firmly, further than a nudge to bring it nearer.
pub const DETACH: f32 = 0.3;

/// How near the surface a wall pulled out of its group must be pushed back, in metres, to snap back where it was.
const PULLED_DEPTH: f32 = 0.1;

/// How near where a wall pulled out of its group stood a snap puts it, in metres, to put it back where it was.
const RETURN: f32 = 0.1;

/// How much nearer the snap already shown ranks against another, as a proportion, so that the preview does not flicker
/// between two neighbours.
const STAY: f32 = 0.8;

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
    /// The knob at its top middle: moved to the right, as the participant sees it, from where it was taken hold of,
    /// it bends the wall around the participant, and back to the left, flattens it, flat near where it bends least;
    /// moving up or down does nothing.
    Bend,
    /// The hinge at the middle of its left edge, on the line it tilts about: moved down from where it was taken hold
    /// of, it tilts the wall's top towards the participant, so that it faces down, and up, tilts it back, upright near
    /// where it tilts least; moving across does nothing.
    Tilt,
    /// The padlock beyond its top left corner, closed while the wall is locked and open while it is not: squeezed by an
    /// owner, it locks or unlocks the wall, for everyone.
    Lock,
    /// Beside the padlock of an environment's own wall unlocked, for an owner: puts it back where and as the environment
    /// has it, locked.
    Reset,
}

/// What a participant may do with walls in the joined space.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rights {
    /// Adjust walls: a participant, in a space whose server takes `SetWall`.
    pub adjust: bool,
    /// Remove walls: an owner, as well.
    pub remove: bool,
    /// Snap walls together, carrying a group as one and taking a wall out of one: in a space whose server snaps them.
    pub snap: bool,
    /// The server locks walls: each says whether it is locked, a locked wall shows its padlock alone, and an
    /// environment's own wall, unlocked, is carried as one someone added is.
    pub locks: bool,
    /// Lock and unlock walls, and put an environment's own back: an owner, where the server locks walls.
    pub lock: bool,
}

/// What a change does to a wall's group.
#[derive(Clone, Debug, PartialEq)]
pub enum Joining {
    /// Snap it beside another, as `attach` says, its group coming with it, or, `alone`, out of its group first.
    Attach { attach: Attach, alone: bool },
    /// Take it out of its group, standing where it was let go.
    Leave,
}

/// Which of the lock's controls a wall taken hold of shows: where the server locks walls, its padlock, which says
/// whether it is locked and, for an owner, locks and unlocks it; and, for an owner, beside the padlock of an
/// environment's own wall unlocked, the control that puts it back where and as the environment has it, locked.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LockControls {
    pub padlock: bool,
    pub reset: bool,
}

impl LockControls {
    /// The lock's controls of `wall` for a participant with `rights`.
    pub fn of(wall: &Wall, rights: Rights) -> Self {
        let padlock = rights.adjust && rights.locks;
        Self {
            padlock,
            reset: padlock
                && rights.lock
                && wall.kind == WallKind::Environment
                && !wall.is_locked(),
        }
    }
}

/// A wall to lock or unlock, or, with `reset`, an environment's own wall to put back as the environment has it, locked,
/// for everyone, as an owner asked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LockChange {
    pub wall: String,
    pub locked: bool,
    pub reset: bool,
}

/// Whether a participant with `rights` carries `wall` when they take hold of it: one someone added, or, where the server
/// locks walls, an environment's own, unless it is locked.
pub fn carried(wall: &Wall, rights: Rights) -> bool {
    !wall.is_locked()
        && (wall.kind == WallKind::Added || (rights.locks && wall.kind == WallKind::Environment))
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
    /// What it does to the wall's group, if anything.
    pub join: Option<Joining>,
    /// How far it tilts from upright, in degrees, `0` for upright.
    pub tilt: Option<f32>,
    /// The wall's version as it was taken hold of, for a change made against how it was, as by a handle, so that the
    /// server refuses it where the wall changed since; `None` for a change that is the participant's own whatever the
    /// wall was, as carrying it, or snapping it.
    pub version: Option<u64>,
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
    /// the knob's and the hinge's drags are measured from.
    origin: Vec3,
    along: f32,
    point: Vec3,
    /// The participant's right as they took hold, level and across the line from their eyes to the handle, which the
    /// knob's drag is measured along.
    right: Vec3,
    /// Whether the knob or the hinge is at one end of its range, or at flat or upright, where it stops, so that reaching
    /// one ticks.
    stopped: bool,
}

/// A wall someone added, or an environment's own unlocked, carried along the ray, as a panel is: the point it was taken
/// hold of stays on the ray, and it keeps facing the way it faced, turned only as the stick turns it, or squared up to
/// face the participant.
#[derive(Clone, Debug, PartialEq)]
struct Carry {
    /// Whether B removes it: one someone added, as an environment's own is never removed.
    removable: bool,
    manipulation: Manipulation,
    /// The way it faced as it was taken hold of, or squared up.
    facing: Quat,
    /// Where it was taken hold of, in its own frame.
    grabbed: Vec3,
    model: Mat4,
    /// How far along the ray it was taken hold of, which pulling it [`DETACH`] nearer than takes it out of its group.
    start: f32,
}

/// Where a wall carried would snap if let go now: the wall of those carried that snaps, the wall it snaps beside, and
/// how.
#[derive(Clone, Debug, PartialEq)]
struct Snapping {
    wall: String,
    to: String,
    snap: group::Snap,
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
    /// The wall whose bar the ray meets, by identity.
    bar: Option<String>,
    /// When B asked to remove the wall carried, which holds panels, so that another press confirms it.
    confirm: Option<Instant>,
    /// The other walls of the group of the wall carried, each as it stood and where it stood from the wall carried.
    members: Vec<(Wall, Mat4)>,
    /// The wall carried was pulled out of its group, so that it comes alone.
    detached: bool,
    /// The walls carried with the one carried, as they are drawn moved with it.
    moved: Vec<Wall>,
    /// Those walls as they were let go, drawn until the server agrees with the wall carried or [`PENDING`] passes.
    moved_pending: Vec<Wall>,
    /// Where the walls carried would snap if let go now.
    snap: Option<Snapping>,
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
    /// The trigger is held: a wall carried snaps to no other, so that let go it stays where it is.
    pub free: bool,
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
    /// A wall to lock, unlock or put back, to send.
    pub lock: Option<LockChange>,
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

/// Which handles `wall` has, for a participant who may adjust it, with the lock's `controls`: the Tomori wall's grid
/// only in its one row, and its slots their own size; and a wall locked, where the server locks walls, its padlock
/// alone, so neither the hinge.
fn handles(wall: &Wall, controls: LockControls) -> Vec<Handle> {
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

/// Where each of `wall`'s handles stands, the wall standing at `frame` and seen from `viewer`, in the space, and how large
/// a handle is drawn, the same for every one: [`HANDLE_ANGLE`] from the eyes at the wall's farthest corner, so that
/// none is smaller. Its corners stand diagonally beyond its corners; the knob above its top edge, at its middle, and the
/// steppers of its columns along that edge towards its right corner; the handle of its columns beside its right edge, at
/// its middle, and the steppers of its rows along that edge towards its bottom corner; the handle of its rows below
/// its bottom edge, at its middle; and the hinge beside its left edge, at its middle, on the line it tilts about, or
/// beyond the edge where it has no room between the corners. Each handle's middle is at least [`SPACING`] handles from every other's, as seen from
/// the viewer, however small the wall, whatever its shape and however obliquely it is seen: where an edge has no room
/// for them, the knob and the steppers move along it, then to a row or column of their own beyond it. The knob and the
/// steppers of its columns stand a row beyond its [`Bar`], which stays where it is whether the wall is held or not.
pub fn handle_points(wall: &Wall, frame: Mat4, viewer: Vec3) -> (Vec<(Handle, Vec3)>, f32) {
    handle_points_among(wall, frame, viewer, &[], None)
}

/// Where each of `wall`'s handles stands, as [`handle_points`] lays them out, with the lock's `controls`: the padlock a
/// row beyond its top left corner, level with the knob, and the control that puts it back a row beyond the padlock,
/// each as far from every other handle; or, where it is locked, the padlock alone, where it would stand.
pub fn handle_points_with(
    wall: &Wall,
    frame: Mat4,
    viewer: Vec3,
    controls: LockControls,
) -> (Vec<(Handle, Vec3)>, f32) {
    handle_points_among_with(wall, frame, viewer, &[], None, controls)
}

/// Where `wall`'s handles stand, as [`handle_points`] lays them out, among `walls` as they are drawn, where it is one of
/// a group: those along an edge another wall of its group is joined across stand along the opposite edge instead, and
/// any that would still stand over the slots of another wall of its group, as large as it is seen, are left out, but
/// for `keep`, the handle being dragged. So none covers what the group shows, however far away it is seen.
pub fn handle_points_among(
    wall: &Wall,
    frame: Mat4,
    viewer: Vec3,
    walls: &[&Wall],
    keep: Option<Handle>,
) -> (Vec<(Handle, Vec3)>, f32) {
    handle_points_among_with(wall, frame, viewer, walls, keep, LockControls::default())
}

/// Where `wall`'s handles stand among `walls`, as [`handle_points_among`] lays them out, with the lock's `controls`, as
/// [`handle_points_with`] places them: the padlock, which says why a wall will not change, is never left out, and
/// stands along the bottom where another wall is joined across its top, and above its top right corner where one is
/// joined across its left edge.
pub fn handle_points_among_with(
    wall: &Wall,
    frame: Mat4,
    viewer: Vec3,
    walls: &[&Wall],
    keep: Option<Handle>,
    controls: LockControls,
) -> (Vec<(Handle, Vec3)>, f32) {
    let mut around = Around::of(wall, walls, keep);
    around.controls = controls;
    let (points, size, _) = lay_out(wall, frame, viewer, &around);
    (points, size)
}

/// Where `wall`'s grab bar stands, the wall standing at `frame` and seen from `viewer`, as [`handle_points`] lays it
/// out with its handles.
pub fn bar(wall: &Wall, frame: Mat4, viewer: Vec3) -> Bar {
    lay_out(wall, frame, viewer, &Around::default()).2
}

/// Where `wall`'s grab bar stands among `walls` as they are drawn, as [`bar`] says, if it shows one: a group shows one
/// bar, above its top left wall, whose slots reach highest, clear of every wall of its group, which carries the whole
/// group as any of its walls does. See `design/projection.md`.
pub fn group_bar(wall: &Wall, frame: Mat4, viewer: Vec3, walls: &[&Wall]) -> Option<Bar> {
    super::boards::bears_bar(wall, walls)
        .then(|| lay_out(wall, frame, viewer, &Around::of(wall, walls, None)).2)
}

/// What stands about a wall of a group, which its handles and bar keep clear of: the slots of the other walls of its
/// group, as they stand on its surface, from its pose; the edges another wall of its group is joined across; the handle
/// being dragged, which stays where it is; and whether it shows its group's bar.
#[derive(Clone, Debug, Default)]
struct Around {
    slots: Vec<group::Extent>,
    joined: Vec<Edge>,
    keep: Option<Handle>,
    /// It shows its group's bar, which then stands over none of those slots.
    bar: bool,
    /// The lock's controls it shows.
    controls: LockControls,
}

impl Around {
    fn of(wall: &Wall, walls: &[&Wall], keep: Option<Handle>) -> Self {
        Self {
            slots: super::boards::members_slots(wall, walls),
            joined: super::boards::joined_edges(wall, walls),
            keep,
            bar: super::boards::bears_bar(wall, walls),
            controls: LockControls::default(),
        }
    }

    /// Whether a box `x` and `y` from a wall's pose on its surface, reaching `half` either way across and up, stands over
    /// none of the slots about it.
    fn clear(&self, x: f32, y: f32, half: (f32, f32)) -> bool {
        self.slots.iter().all(|slots| {
            x + half.0 <= slots.left
                || x - half.0 >= slots.right
                || y + half.1 <= slots.bottom
                || y - half.1 >= slots.top
        })
    }
}

/// A wall's handles and its bar, as [`handle_points_among_with`] and [`group_bar`] lay them out.
fn lay_out(
    wall: &Wall,
    frame: Mat4,
    viewer: Vec3,
    around: &Around,
) -> (Vec<(Handle, Vec3)>, f32, Bar) {
    // Laid out once at the size its corners take, and again at the size its farthest handle then takes, so that none is
    // smaller than its angle: the lock's included, and, with the padlock, the control that puts a wall back, as an
    // environment's own wall unlocked shows it, so that a wall's padlock stands where it does, locked or not.
    let sizing = Around {
        controls: LockControls {
            padlock: around.controls.padlock,
            reset: around.controls.padlock,
        },
        ..around.clone()
    };
    let (points, ..) = lay_out_handles(wall, frame, viewer, None, &sizing);
    let far = points
        .iter()
        .map(|(_, at)| at.distance(viewer))
        .fold(0.3, f32::max);
    let (mut points, size, bar) = lay_out_handles(wall, frame, viewer, Some(far), around);
    // A locked wall's padlock alone, where it stands beside the rest.
    let shown = handles(wall, around.controls);
    points.retain(|(handle, _)| shown.contains(handle));
    (points, size, bar)
}

/// A wall's grab bar: a thin bar a participant who may adjust walls takes hold of the wall by with the grip, as they do
/// its board, so that a wall whose every slot holds a panel, and which is curved, is as easy to take hold of as any.
/// It stands at the middle of the wall's top edge, facing the way the wall faces there, or, for the Tomori wall, below
/// its label, about a third of the wall long. See `design/projection.md`.
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
    /// How far along `ray` it is met, if the ray passes within [`HANDLE_REACH`] of a handle's size of it, in front of
    /// where it begins, as near as a handle is met, so that it is as easy to hold.
    pub fn hit(&self, ray: &Ray) -> Option<f32> {
        // The nearest points of the ray and the bar's middle line, the bar's within its ends.
        let (direction, along) = (ray.direction, self.half);
        let from = ray.origin - self.middle;
        let (b, c) = (direction.dot(along), along.length_squared());
        let (d, e) = (direction.dot(from), along.dot(from));
        let across = c - b * b;
        let s = if across > 1e-9 {
            (e - d * b) / across
        } else {
            0.0
        };
        let s = s.clamp(-1.0, 1.0);
        let t = s * b - d;
        let off = (from + direction * t - along * s).length();
        (t > 0.0 && off <= self.size * HANDLE_REACH).then_some(t)
    }
}

/// How thick a bar is drawn, in handles: thin, though it is met as near as a handle is.
const BAR_THICKNESS: f32 = 0.45;

/// How long a bar is, in handles, as seen, at least and at most, about a third of its wall's width between.
const BAR_LEAST: f32 = 3.0;
const BAR_MOST: f32 = 5.0;

/// How many rows beyond its wall's edge a bar may move out to stand clear of handles, as seen from the side of a wall
/// so curved that it folds, where they are moved on.
const BAR_ROWS: usize = 8;

/// How much further than a handle's [`SPACING`] apart points `a` and `b`, as large as handles `size` across, stand as
/// seen from `viewer` looking `ahead`, across or up, whichever is further, as an angle: at least none, so that the ray
/// meeting one never meets the other, where they stand apart.
fn seen_gap(a: Vec3, b: Vec3, size: f32, viewer: Vec3, ahead: Vec3) -> f32 {
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
    let reach = (across + other) / 2.0 * SPACING;
    (a.x - b.x).abs().max((a.y - b.y).abs()) - reach
}

/// The handles of `wall` and its bar as [`lay_out`] lays them out, sized for a handle `far` from the viewer, or else for
/// its farthest corner.
fn lay_out_handles(
    wall: &Wall,
    frame: Mat4,
    viewer: Vec3,
    far: Option<f32>,
    around: &Around,
) -> (Vec<(Handle, Vec3)>, f32, Bar) {
    // The wall measured along itself, about its middle: a flat wall's board, or a curved wall's slots, each flat, as
    // they are drawn, its middle on the arc and turned to face the line it curves about, so that a point along the wall
    // is on the slot whose column it is in, or across the gap between two, and its edges are where its slots end.
    // Measured along the arc alone, a wall of few wide slots, such as one slot five times a standard one, curved
    // tightly, would have its edges, and the handles beside them, far in front of where its slots end.
    let (width, height) = wall.size();
    let (right, top) = (width / 2.0, height / 2.0);
    let (left, bottom) = (-right, -top);
    let radius = wall.radius();
    let columns = wall.grid().columns as f32;
    let (slot, gap) = (wall.slot_size().0, wall.gap());
    let step = slot + gap;
    // Where `x` along the wall stands, level with its middle, in its own frame, and which way the wall runs there.
    let along = |x: f32| -> (Vec3, Vec3) {
        let Some(radius) = radius else {
            return (Vec3::new(x, 0.0, 0.0), Vec3::X);
        };
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
    };
    // The point of the wall nearest `x` along it and `y` up it, on its edge where they are beyond it, in its frame.
    let on_wall = |x: f32, y: f32| along(x.clamp(left, right)).0 + Vec3::Y * y.clamp(bottom, top);
    let tangent = |x: f32| frame.transform_vector3(along(x.clamp(left, right)).1);
    let samples = [
        (left, top),
        (right, top),
        (left, bottom),
        (right, bottom),
        (0.0, 0.0),
    ];
    let far = samples
        .iter()
        .map(|(x, y)| frame.transform_point3(on_wall(*x, *y)).distance(viewer))
        .fold(far.unwrap_or(0.3), f32::max);
    let size = HANDLE_ANGLE * far;
    // A handle's worth along the wall and up it, longer as the wall is seen more obliquely, where it is seen most
    // obliquely, so that the handles stand as far apart to the eye wherever they are along it.
    let least = |axis: &dyn Fn(f32) -> Vec3| {
        samples
            .iter()
            .map(|(x, y)| {
                let towards =
                    (frame.transform_point3(on_wall(*x, *y)) - viewer).normalize_or(Vec3::NEG_Z);
                let axis = axis(*x).normalize_or(Vec3::X);
                (1.0 - axis.dot(towards).powi(2)).max(0.0).sqrt()
            })
            .fold(1.0, f32::min)
            .max(OBLIQUE)
    };
    let up = frame.transform_vector3(Vec3::Y);
    let (across_seen, up_seen) = (least(&tangent), least(&|_| up));
    let (u, v) = (size / across_seen, size / up_seen);
    // Above or below an edge, a handle stands off it square to the line the eyes see it along, as far as it is seen
    // to, so that above the top of a wall seen receding, as a wall tilted back is, it stands near the edge rather than
    // far up where the wall would go on, each row further out seen as less apart. Beyond its sides, a handle stands
    // along the slot at that end, as the wall would go on, so that it stands clear of the slots it is beside.
    let above = |at: Vec3| {
        let sight = (at - viewer).normalize_or(Vec3::NEG_Z);
        (up - sight * up.dot(sight)).normalize_or(up)
    };
    let beside = |side: f32| tangent(side * right).normalize_or(Vec3::X) * side;
    // Where each handle stands, along the wall and up it, put in the space once it is chosen: on the wall, or beyond
    // its nearest edge, along the slot at its side, and above or below, as far as it is seen to be.
    let at = |x: f32, y: f32| (x, y);
    let put = |(x, y): (f32, f32)| {
        let (across, upward) = (x - x.clamp(left, right), y - y.clamp(bottom, top));
        let edge = frame.transform_point3(on_wall(x, y)) + tangent(x) * across;
        edge + above(edge) * upward * up_seen
    };
    let (xl, xr) = (left - OUTSET * u, right + OUTSET * u);
    let (yt, yb) = (top + OUTSET * v, bottom - OUTSET * v);
    let (pu, pv) = (SPACING * u, SPACING * v);
    let between = |preferred: f32, low: f32, high: f32| preferred.max(low).min(high);
    let room = xr - xl;
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
        (columns, (0.0, 0.0), (0.0, 0.0))
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
    // In a group, the handles along an edge another wall of its group is joined across stand along the opposite edge,
    // where no wall is joined, and nowhere otherwise, and a corner beside a joined edge is left out, so that none stands
    // in the gutter between two walls, over the other's slots: `Some(true)` where they stand opposite.
    let joined = |edge: Edge| around.joined.contains(&edge);
    let side = |edge: Edge| match (joined(edge), joined(edge.opposite())) {
        (false, _) => Some(false),
        (true, false) => Some(true),
        (true, true) => None,
    };
    let corner = |x: i8, y: i8| {
        let (across, up) = (
            if x < 0 { Edge::Left } else { Edge::Right },
            if y > 0 { Edge::Top } else { Edge::Bottom },
        );
        (!joined(across) && !joined(up))
            .then(|| at(if x < 0 { xl } else { xr }, if y > 0 { yt } else { yb }))
    };
    let (right_side, top_side, bottom_side) =
        (side(Edge::Right), side(Edge::Top), side(Edge::Bottom));
    let mirrored = |(x, y): (f32, f32)| (-x, y);
    // Down the left, the hinge at the middle, on the line the wall tilts about, so that it stays under the ray as the
    // wall tilts, or beyond the edge where there is no room between the corners. The Tomori wall has no corners; a wall
    // in a group, which stands upright, has no hinge.
    // Laid out as the wall is unlocked, so that a locked wall's padlock stands where it would beside the rest.
    let unlocked = wall.clone().with_locked(false);
    let tilts = handles(&unlocked, LockControls::default()).contains(&Handle::Tilt);
    let hinge = if tomori || span >= 2.0 * pv - 1e-6 {
        at(xl, centre)
    } else {
        at(xl - pu, centre)
    };
    // The grab bar, at the middle of its top edge, or, for the Tomori wall, whose top the status pill stands just above
    // from home, below its label: a third of the wall long, within bounds, as long as it may be and stand a handle's
    // spacing, as seen, from the handles beside it, or else a row further out, where it has room.
    let middle = frame.transform_point3(Vec3::ZERO);
    let ahead = (middle - viewer).normalize_or(Vec3::NEG_Z);
    let fixed: Vec<(f32, f32)> = if tomori {
        vec![columns, hinge]
    } else {
        let corners = [(-1, 1), (1, 1), (-1, -1), (1, -1)]
            .into_iter()
            .filter_map(|(x, y)| corner(x, y));
        let right = right_side.into_iter().flat_map(|opposite| {
            [columns, row_fewer, row_more]
                .map(|point| if opposite { mirrored(point) } else { point })
        });
        corners
            .chain(right)
            .chain((bottom_side == Some(false)).then_some(rows))
            .chain(tilts.then_some(hinge))
            .collect()
    };
    let fixed: Vec<Vec3> = fixed.into_iter().map(put).collect();
    let below = tomori;
    // A handle's worth along the bar, as foreshortened where it stands, at the wall's middle.
    let towards = (middle - viewer).normalize_or(Vec3::NEG_Z);
    let across = tangent(0.0).normalize_or(Vec3::X);
    let ub = size
        / (1.0 - across.dot(towards).powi(2))
            .max(0.0)
            .sqrt()
            .max(OBLIQUE);
    let label = match radius {
        Some(_) if below => super::ARC_LABEL_DROP + super::arc_label(wall).size().1 / 2.0,
        _ => 0.0,
    };
    let first = if below {
        bottom - label - OUTSET * v
    } else {
        yt
    };
    let outward = if below { -pv } else { pv };
    let (least, most) = (BAR_LEAST * ub / 2.0, BAR_MOST * ub / 2.0);
    let preferred = (width / 6.0).clamp(least, most);
    // How far apart from `fixed` a bar `half` long either way along `row` stands, as seen, as [`seen_gap`] says of
    // the nearest.
    let gap = |fixed: &[Vec3], row: f32, half: f32| {
        let samples = ((2.0 * half / (0.2 * size)).ceil() as usize).max(1);
        (0..=samples)
            .flat_map(|sample| {
                let x = -half + 2.0 * half * sample as f32 / samples as f32;
                let point = put(at(0.0, row)) + frame.transform_vector3(Vec3::X * x);
                fixed
                    .iter()
                    .map(move |handle| seen_gap(point, *handle, size, viewer, ahead))
            })
            .fold(f32::INFINITY, f32::min)
    };
    let clear = |fixed: &[Vec3], row: f32, half: f32| {
        // The bar its group shows stands over none of its group's slots, as thick as it is drawn.
        (!around.bar || around.clear(0.0, row, (half, BAR_THICKNESS * size / 2.0)))
            && gap(fixed, row, half) >= 0.0
    };
    // As long as it may be, nearest the wall, that stands clear of `fixed`; or else, where none does, as seen from
    // beside the end of a wall curved around the participant, whose handles there are near and large, the one that
    // stands furthest from them.
    let choose = |fixed: &[Vec3]| {
        let candidates = (0..BAR_ROWS).flat_map(|lift| {
            (0..=4).map(move |step| {
                (
                    first + outward * lift as f32,
                    preferred + (least - preferred) * step as f32 / 4.0,
                )
            })
        });
        candidates
            .clone()
            .find(|(row, half)| clear(fixed, *row, *half))
            .or_else(|| {
                candidates.max_by(|a, b| gap(fixed, a.0, a.1).total_cmp(&gap(fixed, b.0, b.1)))
            })
            .unwrap_or((first, least))
    };
    let (bar_row, bar_half) = choose(&fixed);
    // Along the top, a row beyond the bar where it stands there: the corners, the knob at the middle where it has room,
    // and the steppers towards the right corner.
    let beyond = |row: f32| if below { yt } else { row + pv };
    let yk = beyond(bar_row);
    let (knob, fewer, more) = if room >= 4.0 * pu - 1e-6 {
        (
            at(between(0.0, xl + pu, xr - 3.0 * pu), yk),
            at(xr - 2.0 * pu, yk),
            at(xr - pu, yk),
        )
    } else if room >= 2.0 * pu - 1e-6 {
        (
            at(between(0.0, xl + pu, xr - pu), yk),
            at(xr - pu, yk + pv),
            at(xr, yk + pv),
        )
    } else {
        (
            at(xr - 2.0 * pu, yk + pv),
            at(xr - pu, yk + pv),
            at(xr, yk + pv),
        )
    };
    // Each handle, and whether it stands along the edge opposite its own: those of the top a row below the handle of its
    // rows, those of the right edge along the left, and the handle of its rows a row beyond the knob. The padlock stands a
    // row beyond the top left corner, level with the knob, and the control that puts the wall back a row beyond it: along
    // the bottom where the top is joined, as the knob is, and above the top right corner where the left edge is joined.
    let left_side = side(Edge::Left);
    let local = handles(&unlocked, around.controls)
        .into_iter()
        .filter_map(|handle| {
            let lock = |row: f32| {
                let right = left_side == Some(true);
                let mut point = at(if right { -xl } else { xl }, row);
                if top_side == Some(true) {
                    point.1 = -point.1 - pv;
                }
                (point, right)
            };
            let (point, opposite) = match handle {
                Handle::Corner(x, y) => (corner(x, y)?, false),
                Handle::Columns => (columns, right_side?),
                Handle::Rows => (rows, bottom_side?),
                Handle::Step(Axis::Columns, step) => {
                    (if step < 0 { fewer } else { more }, top_side?)
                }
                Handle::Step(Axis::Rows, step) => {
                    (if step < 0 { row_fewer } else { row_more }, right_side?)
                }
                Handle::Bend => (knob, top_side?),
                Handle::Tilt => (hinge, false),
                Handle::Lock => lock(yk),
                Handle::Reset => lock(yk + pv),
            };
            let point = match handle {
                _ if !opposite => point,
                Handle::Bend | Handle::Step(Axis::Columns, _) => (point.0, -point.1 - pv),
                Handle::Rows => (point.0, yk + pv),
                Handle::Lock | Handle::Reset => point,
                _ => mirrored(point),
            };
            // Which way is away from the wall, along the edge it stands by: back along it where it stands opposite.
            let away = if opposite { -1.0 } else { 1.0 };
            // Whether it stands a row beyond the bar, and moves with it.
            let lifts = match handle {
                Handle::Bend | Handle::Step(Axis::Columns, _) => !opposite,
                Handle::Lock | Handle::Reset => top_side != Some(true),
                _ => false,
            };
            Some(((handle, put(point)), (away, lifts)))
        });
    let (mut points, ways): (Vec<_>, Vec<(f32, bool)>) = local.unzip();
    let (away, lifts): (Vec<f32>, Vec<bool>) = ways.into_iter().unzip();
    // A wall curved so far that it folds as seen from the side may still bring two together: the one that is not a
    // corner moves on, beyond its edge, until none overlaps.
    let outward = |handle: Handle, away: f32, at: Vec3| {
        let spacing = SPACING * size;
        match handle {
            Handle::Bend | Handle::Step(Axis::Columns, _) => above(at) * spacing * away,
            Handle::Rows => -above(at) * spacing * away,
            Handle::Tilt => beside(-1.0) * pu,
            // Beyond the left edge, or the right where it stands there.
            Handle::Lock | Handle::Reset => beside(-away) * pu,
            _ => beside(away) * pu,
        }
    };
    let rank = |handle: Handle| match handle {
        Handle::Corner(..) => 0,
        Handle::Columns | Handle::Rows | Handle::Tilt => 1,
        Handle::Bend => 2,
        Handle::Step(..) => 3,
        Handle::Lock => 4,
        Handle::Reset => 5,
    };
    let separate = |points: &mut Vec<(Handle, Vec3)>| {
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
                    let by = outward(points[moving].0, away[moving], points[moving].1);
                    points[moving].1 += by;
                    moved = true;
                }
            }
            if !moved {
                break;
            }
        }
    };
    separate(&mut points);
    // Handles moved on may come in line with the bar, seen from the side of a wall so curved that it folds: it is then
    // chosen again clear of them where they stand, and the knob and the steppers of its columns, beyond it, move with it.
    let top = lifts;
    let others: Vec<Vec3> = points
        .iter()
        .zip(&top)
        .filter(|(_, top)| !**top)
        .map(|((_, at), _)| *at)
        .collect();
    let (bar_row, bar_half) = if clear(&others, bar_row, bar_half) {
        (bar_row, bar_half)
    } else {
        let (row, half) = choose(&others);
        for ((_, at), top) in points.iter_mut().zip(&top) {
            if *top {
                *at += above(*at) * (beyond(row) - yk) * up_seen;
            }
        }
        (row, half)
    };
    // Seen steeply, as a very tall wall's top is from below and to the side, a row is foreshortened more beyond the
    // wall than at its corners, so that the knob and the steppers of its columns may still come near the bar: they move
    // on, beyond it, until they stand clear of it as seen, with the handle of its rows where it stands beyond the knob.
    let lifted: Vec<bool> = points
        .iter()
        .zip(&away)
        .zip(&top)
        .map(|(((handle, _), away), top)| *top || (*handle == Handle::Rows && *away < 0.0))
        .collect();
    for _ in (0..BAR_ROWS).filter(|_| !below) {
        let beyond: Vec<Vec3> = points
            .iter()
            .zip(&lifted)
            .filter(|(_, lifted)| **lifted)
            .map(|((_, at), _)| *at)
            .collect();
        if clear(&beyond, bar_row, bar_half) {
            break;
        }
        for ((_, at), lifted) in points.iter_mut().zip(&lifted) {
            if *lifted {
                *at += above(*at) * pv * up_seen;
            }
        }
    }
    // Moved beyond the bar, they may come near a corner of a wall so curved that it folds as seen: they move on again,
    // outward, which only takes them further from the bar.
    separate(&mut points);
    // Last, in a group, none stands over another wall's slots, as large as it is seen, but the handle being dragged and
    // the padlock, which says why a wall will not change.
    if !around.slots.is_empty() {
        let base = pose(frame);
        let half = (u / 2.0, v / 2.0);
        points.retain(|(handle, at)| {
            let (x, y, ..) = group::on_surface(
                &base,
                radius,
                &Pose {
                    position: at.to_array(),
                    orientation: base.orientation,
                },
            );
            around.keep == Some(*handle) || *handle == Handle::Lock || around.clear(x, y, half)
        });
    }
    // Turned as the wall is, where it stands.
    let middle = put(at(0.0, bar_row));
    let bar = Bar {
        middle,
        half: frame.transform_vector3(Vec3::X * bar_half),
        size,
        model: Mat4::from_translation(middle - frame.transform_point3(Vec3::ZERO)) * frame,
    };
    (points, size, bar)
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
    handle_point_among(wall, frame, handle, viewer, &[], None)
}

/// Where `handle` of `wall` is drawn among `walls`, as [`handle_points_among`] lays them out, keeping `keep`, and how
/// large, or the wall's middle for a handle it does not show.
pub fn handle_point_among(
    wall: &Wall,
    frame: Mat4,
    handle: Handle,
    viewer: Vec3,
    walls: &[&Wall],
    keep: Option<Handle>,
) -> (Vec3, f32) {
    let (points, size) = handle_points_among(wall, frame, viewer, walls, keep);
    let at = points
        .into_iter()
        .find(|(shown, _)| *shown == handle)
        .map_or(frame.transform_point3(Vec3::ZERO), |(_, at)| at);
    (at, size)
}

/// What `handle` shows as text: the knob's dot, which its glyph centres on its cell; every other symbol is drawn as
/// shapes, by [`symbol`], centred on its handle, as a glyph's ink, sitting on its baseline, is not.
fn glyph(handle: Handle) -> &'static str {
    match handle {
        Handle::Bend => "●",
        _ => " ",
    }
}

/// How thick the lines of a handle's symbol are, in font pixels: as a bold glyph's.
const STROKE: f32 = 1.3;

/// What `handle` shows drawn as shapes, in font pixels from its cell's top left corner, centred on the cell, and so on
/// the handle: the arrows of its edges, across or down; a stepper's minus or plus; the hinge; the padlock, closed while
/// its wall is `locked`; and a circle's arrow turning back for the reset. `behind` is the colour behind it, which
/// hides what a shape leaves out.
fn symbol(handle: Handle, locked: bool, behind: [f32; 4]) -> Vec<crate::shapes::Shape> {
    use crate::shapes::Shape;
    use crate::ui_text::{CELL_HEIGHT, CELL_WIDTH};
    let middle = Vec2::new(CELL_WIDTH as f32 / 2.0, CELL_HEIGHT as f32 / 2.0);
    let rule = |from: Vec2, to: Vec2| Shape::Rule {
        from: (middle + from).to_array(),
        to: (middle + to).to_array(),
        width: STROKE,
        color: BRIGHT,
    };
    // An arrow from end to end through the middle, along `along`, with a head at each end.
    let arrow = |along: Vec2| {
        let (tip, side) = (along * 4.5, along.perp() * 2.5);
        vec![
            rule(-tip, tip),
            rule(tip, tip - along * 2.5 + side),
            rule(tip, tip - along * 2.5 - side),
            rule(-tip, -tip + along * 2.5 + side),
            rule(-tip, -tip + along * 2.5 - side),
        ]
    };
    match handle {
        Handle::Columns => arrow(Vec2::X),
        Handle::Rows => arrow(Vec2::Y),
        Handle::Step(_, -1) => vec![rule(Vec2::new(-3.0, 0.0), Vec2::new(3.0, 0.0))],
        Handle::Step(..) => vec![
            rule(Vec2::new(-3.0, 0.0), Vec2::new(3.0, 0.0)),
            rule(Vec2::new(0.0, -3.0), Vec2::new(0.0, 3.0)),
        ],
        Handle::Tilt => hinge_shapes(),
        Handle::Lock => padlock(locked, behind),
        // A ring, its quarter at the top right left out, and a head at its right end, pointing up, the way it turns.
        // Its ring a little left of the middle, so that with its head it centres.
        Handle::Reset => {
            let (radius, left) = (3.6, Vec2::X * 0.9);
            let line = radius / 3.0;
            let centre = middle - left;
            let tip = Vec2::new(radius - line / 2.0, -1.6) - left;
            vec![
                Shape::Ring {
                    centre: centre.to_array(),
                    radius,
                    color: BRIGHT,
                },
                Shape::Fill {
                    x: centre.x + 0.3,
                    y: centre.y - radius - 0.5,
                    width: radius + 0.5,
                    height: radius - 0.3,
                    color: behind,
                },
                rule(tip, tip + Vec2::new(-2.2, 2.6)),
                rule(tip, tip + Vec2::new(2.2, 2.6)),
            ]
        }
        Handle::Corner(..) | Handle::Bend => Vec::new(),
    }
}

/// The colour of a padlock closed, warm, so that a locked wall's is told from an open one's at a glance.
pub const LOCKED: [f32; 4] = [1.0, 0.8, 0.38, 1.0];

/// A handle's face, `size` across, highlighted while the ray meets it or it is dragged; the padlock closed while its
/// wall is `locked`, and open while it is not. Its symbol is centred on it, by what it draws.
pub fn handle_face(handle: Handle, size: f32, highlighted: bool, locked: bool) -> Canvas {
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
    let [red, green, blue, _] = canvas.background;
    canvas.shapes = symbol(handle, locked, [red, green, blue, 1.0]);
    let (width, _) = canvas.size_in_pixels();
    canvas.pixel = size / width as f32;
    canvas
}

/// The hinge's face, in font pixels from its cell's top left corner, inside its canvas's padding, as every shape is:
/// the line the wall tilts about, level through the cell's middle and out into the padding, dim, and the wall's face
/// tilted across it, bright, as a door is drawn on its hinge, so that it is centred on its handle.
fn hinge_shapes() -> Vec<crate::shapes::Shape> {
    use crate::shapes::Shape;
    use crate::ui_text::{CELL_HEIGHT, CELL_WIDTH};
    let middle = [CELL_WIDTH as f32 / 2.0, CELL_HEIGHT as f32 / 2.0];
    vec![
        Shape::Rule {
            from: [middle[0] - 6.0, middle[1]],
            to: [middle[0] + 6.0, middle[1]],
            width: 1.0,
            color: DIM,
        },
        Shape::Rule {
            from: [middle[0] - 3.0, middle[1] + 5.0],
            to: [middle[0] + 3.0, middle[1] - 5.0],
            width: 1.6,
            color: BRIGHT,
        },
        Shape::Disc {
            centre: middle,
            radius: 1.6,
            color: BRIGHT,
        },
    ]
}

/// A padlock in a cell, of shapes, in font pixels from its top left corner: a body below and a shackle above it, closed
/// into the body while `locked`, in [`LOCKED`]'s warm colour, and, unlocked, raised out of it on one side, in the
/// handles' bright one. `behind` is the colour behind it, which hides the shackle's ring below its legs.
pub fn padlock(locked: bool, behind: [f32; 4]) -> Vec<crate::shapes::Shape> {
    use crate::shapes::Shape;
    use crate::ui_text::{CELL_HEIGHT, CELL_WIDTH};
    let (width, height) = (CELL_WIDTH as f32, CELL_HEIGHT as f32);
    let color = if locked { LOCKED } else { BRIGHT };
    // The body, a little rounded, across most of the cell's lower half, raised a little so that, heavier than the
    // shackle above it, the whole centres on the cell, and so on its handle.
    let body = (1.0, height * 0.5 - 1.5, width - 2.0, height * 0.42);
    let radius = (body.2 - 1.6) / 2.0;
    let middle = width / 2.0;
    // The shackle's top, a ring, its lower half hidden, with a leg down each side: open, it is raised, its right leg
    // short of the body.
    let lift = if locked { 0.0 } else { 2.0 };
    let top = body.1 - 0.4 - lift;
    let line = radius / 3.0;
    let (left, right) = (middle - radius + line / 2.0, middle + radius - line / 2.0);
    let right_end = if locked { body.1 + 0.5 } else { top + 0.8 };
    vec![
        Shape::Ring {
            centre: [middle, top],
            radius,
            color,
        },
        Shape::Fill {
            x: middle - radius - 0.5,
            y: top,
            width: 2.0 * radius + 1.0,
            height: radius + 0.5,
            color: behind,
        },
        Shape::Rule {
            from: [left, top - 0.05],
            to: [left, body.1 + 0.5],
            width: line,
            color,
        },
        Shape::Rule {
            from: [right, top - 0.05],
            to: [right, right_end],
            width: line,
            color,
        },
        Shape::Rect {
            x: body.0,
            y: body.1,
            width: body.2,
            height: body.3,
            color,
            radius: 0.8,
            outline: None,
        },
        // Its keyhole, dark.
        Shape::Disc {
            centre: [middle, body.1 + body.3 * 0.42],
            radius: 0.7,
            color: behind,
        },
    ]
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
        && width <= layout::MOST_SIZE.0
        && height <= layout::MOST_SIZE.1
        && (wall.kind != WallKind::Tomori
            || (wall.grid().rows == 1 && wall.grid().columns <= TOMORI_COLUMNS))
        && match wall.shape {
            WallShape::Arc { .. } => wall.radius().is_some(),
            _ => true,
        }
}

/// How far `moved` metres from where a handle was taken hold of counts, past [`DEAD_ZONE`] either way, as a proportion
/// of [`TRAVEL`]: 0 within it, and continuous beyond it, so that leaving it does not jump.
fn moved_by(moved: f32) -> f32 {
    if !moved.is_finite() {
        return 0.0;
    }
    moved.signum() * (moved.abs() - DEAD_ZONE).max(0.0) / TRAVEL
}

/// How much of a range, from its start, flat or upright, to either end, a handle `along` of the way there takes: half
/// as fast as evenly at its start, so that a little is fine to set, and half as fast again at its end, so that the end
/// still comes within [`TRAVEL`]; never less for further, and odd, so that either way is alike.
pub fn ease(along: f32) -> f32 {
    along * (1.0 + along.abs()) / 2.0
}

/// How far of the way along a range a handle is to take `taken` of it: the inverse of [`ease`].
pub fn unease(taken: f32) -> f32 {
    taken.signum() * ((1.0 + 8.0 * taken.abs()).sqrt() - 1.0) / 2.0
}

/// The radius `wall`, as it was when its knob was taken hold of, bends to when the hand has moved `moved` metres to the
/// right, as the participant sees it, from where it took hold of it, or flat: the knob is as far along its range as the
/// wall's curvature, one over its radius, is [`ease`]d of [`most_bend`], and moves along it with the hand, from flat to
/// the most over [`TRAVEL`], so that moving further right bends it more and back left towards where it was taken, and
/// beyond, bends it less, never beyond either. Within [`DEAD_ZONE`] of where it was taken, it is as it was; bending
/// less than [`FLAT_BEND`] of the most it may, unless it was bent less than twice that, it is flat. A radius is a whole
/// number of centimetres, rounded so that the wall stays within half a circle.
pub fn bent(wall: &Wall, moved: f32) -> Option<f32> {
    let start = wall.radius();
    let moved = moved_by(moved);
    if moved == 0.0 {
        return start;
    }
    let most = most_bend(wall);
    let was = start.map_or(0.0, |radius| 1.0 / radius);
    let along = (unease(was / most) + moved).clamp(0.0, 1.0);
    let curvature = most * ease(along);
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

/// How far `wall`, as it was when its hinge was taken hold of, tilts, in degrees, when the hand has moved `moved` metres
/// down from where it took hold of it: as it was, more, its top towards the participant, as the hand moves down, and
/// less, then back, as it moves up, the hinge as far along its range as its tilt is [`ease`]d of [`layout::MOST_TILT`]
/// either way, from one end of [`layout::TILTS`] to the other over [`TRAVEL`], never beyond either. Within
/// [`DEAD_ZONE`] of where it was taken, it is as it was; within [`UPRIGHT_SNAP`] of upright, it is upright. A tilt is a
/// whole number of degrees.
pub fn tilted(wall: &Wall, moved: f32) -> f32 {
    let start = wall.tilt();
    let moved = moved_by(moved);
    if moved == 0.0 {
        return start;
    }
    let along = (unease(start / layout::MOST_TILT) + 2.0 * moved).clamp(-1.0, 1.0);
    let tilt = (layout::MOST_TILT * ease(along)).clamp(layout::TILTS.0, layout::TILTS.1);
    if tilt.abs() < UPRIGHT_SNAP {
        0.0
    } else {
        tilt.round()
    }
}

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
        self.members.clear();
        self.moved.clear();
        self.detached = false;
        self.snap = None;
        self.confirm = None;
        self.hovered = None;
        self.bar = None;
        self.wall.take().is_some()
    }

    /// The server refused the change sent to the wall `wall`, as made against a version it has changed since: drawn as
    /// the server has it at once, rather than as sent until [`PENDING`] passes, as the projection sent again says.
    pub fn refused(&mut self, wall: &str) {
        if self
            .pending
            .as_ref()
            .is_some_and(|(sent, _)| sent.id == wall)
        {
            self.pending = None;
            self.moved_pending.clear();
        }
    }

    /// B pressed while carrying a wall, with `held` panels on it, at `now`: the wall to remove, for an owner, or `None`
    /// when it holds panels and this is not a second press within [`CONFIRM`], which it then waits for.
    pub fn remove(&mut self, held: usize, now: Instant) -> Option<String> {
        // Only a wall someone added, carried, is removed.
        let wall = self
            .carry
            .as_ref()
            .filter(|carry| carry.removable)
            .and(self.wall.clone())?;
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
                    .iter()
                    .chain(&self.moved)
                    .chain(self.pending.as_ref().map(|(pending, _)| pending))
                    .chain(&self.moved_pending)
                    .find(|shown| shown.id == wall.id);
                shown.cloned().unwrap_or_else(|| wall.clone())
            })
            .collect()
    }

    /// Whether the wall carried would snap beside another if let go now.
    pub fn snapping(&self) -> bool {
        self.snap.is_some()
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
                    && (wall.tilt() - sent.tilt()).abs() < 0.05
                    && Vec3::from_array(wall.pose.position)
                        .distance(Vec3::from_array(sent.pose.position))
                        < 0.01
                    && wall.is_locked() == sent.is_locked()
            });
            if agreed || input.now.saturating_duration_since(*since) >= PENDING {
                self.pending = None;
                self.moved_pending.clear();
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
        // Locked meanwhile, as by an owner elsewhere, a wall is no longer dragged or carried, but still held, so that its
        // padlock says why.
        if input.rights.locks
            && (self.drag.is_some() || self.carry.is_some())
            && self
                .wall
                .as_ref()
                .and_then(|wall| input.projection.wall(wall))
                .is_some_and(Wall::is_locked)
        {
            let held = self.wall.take();
            self.stop();
            self.wall = held;
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
        let all: Vec<&Wall> = walls.iter().map(|(wall, _)| *wall).collect();
        if let (Some((wall, frame)), Some(ray), None, None) =
            (&editing, ray, &self.drag, &self.carry)
        {
            let controls = LockControls::of(wall, input.rights);
            let (points, size) =
                handle_points_among_with(wall, *frame, input.viewer, &all, None, controls);
            met = nearest_handle(&points, size, &ray);
        }
        let hovered = self
            .drag
            .as_ref()
            .map(|drag| drag.handle)
            .or(met.map(|(handle, _)| handle));
        // The bar the ray meets, where it meets no handle, nearest along it, for a participant who may adjust walls.
        let bar_met = ray
            .filter(|_| {
                input.rights.adjust && met.is_none() && self.drag.is_none() && self.carry.is_none()
            })
            .and_then(|ray| {
                walls
                    .iter()
                    .filter_map(|(wall, frame)| {
                        group_bar(wall, *frame, input.viewer, &all)?
                            .hit(&ray)
                            .map(|along| (wall.id.clone(), along))
                    })
                    .min_by(|a, b| a.1.total_cmp(&b.1))
            });
        let on_bar = bar_met.as_ref().map(|(wall, _)| wall.clone());
        output.ticked = (hovered.is_some() && hovered != self.hovered)
            || (on_bar.is_some() && on_bar != self.bar);
        self.hovered = hovered;
        self.bar = on_bar;
        // A handle or a bar the ray meets is met before a panel behind it.
        output.reach = met
            .map(|(_, along)| along)
            .or(bar_met.as_ref().map(|(_, along)| *along));
        // A squeeze takes the handle the ray meets, or, failing anything nearer, the wall it meets.
        if input.pressed && input.active && input.rights.adjust {
            if let (Some((handle, along)), Some((wall, frame)), Some(ray)) = (met, &editing, ray)
                && along < input.nearer
            {
                output.took = true;
                match handle {
                    // The padlock locks or unlocks the wall, for an owner, shown so at once; for anyone else it only
                    // says whether it is locked. Putting it back moves it where the server says, so it is let go.
                    Handle::Lock | Handle::Reset => {
                        if input.rights.lock {
                            let reset = handle == Handle::Reset;
                            let locked = reset || !wall.is_locked();
                            output.lock = Some(LockChange {
                                wall: wall.id.clone(),
                                locked,
                                reset,
                            });
                            if reset {
                                self.stop();
                                self.pending = None;
                            } else {
                                self.pending = Some((wall.clone().with_locked(locked), input.now));
                            }
                        }
                    }
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
                                join: None,
                                tilt: None,
                                version: Some(wall.version),
                            });
                            self.pending = Some((changed, input.now));
                        }
                    }
                    handle => {
                        let grab =
                            on_plane(*frame, &ray).map_or(Vec2::ONE, |at| Vec2::new(at.x, at.y));
                        let point = ray.at(along);
                        // Seen from straight above or below, the wall's own right.
                        let right = (point - input.viewer)
                            .with_y(0.0)
                            .cross(Vec3::Y)
                            .normalize_or(
                                frame
                                    .transform_vector3(Vec3::X)
                                    .with_y(0.0)
                                    .normalize_or(Vec3::X),
                            );
                        self.drag = Some(Drag {
                            handle,
                            start: wall.clone(),
                            grab,
                            origin: ray.origin,
                            along,
                            point,
                            right,
                            // Taken hold of where it is, it ticks only once it reaches a stop again.
                            stopped: true,
                        });
                    }
                }
            } else if let Some(ray) = ray.filter(|_| self.drag.is_none() && self.carry.is_none()) {
                // The wall's board, an empty slot or its bar nearer than anything else: it is taken hold of, and one
                // someone added is carried.
                let taken = walls
                    .iter()
                    .filter_map(|(wall, frame)| {
                        let bar = bar_met
                            .as_ref()
                            .filter(|(on, _)| *on == wall.id)
                            .map(|(_, along)| *along);
                        super::wall_hit(wall, *frame, &ray)
                            .into_iter()
                            .chain(bar)
                            .min_by(f32::total_cmp)
                            .map(|along| (*wall, *frame, along))
                    })
                    .filter(|(_, _, along)| *along < input.nearer && *along < input.panel)
                    .min_by(|a, b| a.2.total_cmp(&b.2));
                match taken {
                    Some((wall, frame, along)) => {
                        output.took = true;
                        output.ticked = true;
                        self.wall = Some(wall.id.clone());
                        self.confirm = None;
                        // A group with a wall locked in it stays where it is, as that wall would move with it.
                        let anchored = wall.group.is_some()
                            && walls.iter().any(|(other, _)| {
                                other.id != wall.id
                                    && other.group == wall.group
                                    && other.is_locked()
                            });
                        if carried(wall, input.rights) && !anchored {
                            let (_, facing, _) = frame.to_scale_rotation_translation();
                            self.carry = Some(Carry {
                                removable: wall.kind == WallKind::Added,
                                manipulation: Manipulation::new(along),
                                facing,
                                grabbed: frame.inverse().transform_point3(ray.at(along)),
                                model: frame,
                                start: along,
                            });
                            self.preview = Some(wall.clone());
                            // Its group comes with it, each wall standing where it stood from it.
                            let group = wall.group.as_ref().filter(|_| input.rights.snap);
                            self.members = walls
                                .iter()
                                .filter(|(other, _)| {
                                    other.id != wall.id
                                        && group.is_some()
                                        && other.group.as_ref() == group
                                })
                                .map(|(other, at)| ((*other).clone(), frame.inverse() * *at))
                                .collect();
                            self.moved = self
                                .members
                                .iter()
                                .map(|(other, _)| other.clone())
                                .collect();
                            self.detached = false;
                            self.snap = None;
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
        if let (Some(drag), Some(ray)) = (&mut self.drag, input.ray) {
            // Where it stood as the handle was taken hold of, which is shown, as one no longer shown is let go of.
            let frame = model(&drag.start.frame()).unwrap_or_default();
            let start = &drag.start;
            // How far the hand has moved from where it took hold of the knob or the hinge; in the desktop window,
            // whose ray begins at the eyes, how far the point it was taken at has moved. The hinge counts how far down,
            // and the knob how far to the right, as the participant saw it as they took hold, so that moving the other
            // way, or towards the wall, does nothing.
            let moved = match input.controls {
                Controls::Keyboard => ray.at(drag.along) - drag.point,
                Controls::Controllers => ray.origin - drag.origin,
            };
            let down = -moved.y;
            let right = moved.dot(drag.right);
            let changed = match drag.handle {
                // A corner sizes its slots' width and height apart, each as far across as the ray now meets the wall's
                // plane against where it met it as it was taken hold of, to the power of [`CORNER_GAIN`], the wall
                // growing about its middle, so that a standard slot grows to the largest in one drag.
                Handle::Corner(..) => on_plane(frame, &ray).map(|at| {
                    let (width, height) = start.slot_size();
                    let across = (at.x.abs() / drag.grab.x.abs().max(0.05)).powf(CORNER_GAIN);
                    let down = (at.y.abs() / drag.grab.y.abs().max(0.05)).powf(CORNER_GAIN);
                    let (scale, aspect, _) = slot_shape(width * across, height * down);
                    laid_out(start, start.grid(), scale, aspect, start.radius())
                }),
                Handle::Columns => on_plane(frame, &ray)
                    .map(|at| with_grid(start, nearest_grid(start, Axis::Columns, at.abs()))),
                Handle::Rows => on_plane(frame, &ray).map(|at| {
                    let at = Vec3::new(at.x, -at.y.abs(), at.z);
                    with_grid(start, nearest_grid(start, Axis::Rows, at))
                }),
                // The hinge tilts it as far as the hand has moved down, its top towards the participant.
                Handle::Tilt => {
                    let tilt = tilted(start, down);
                    Some(start.clone().with_tilt((tilt != 0.0).then_some(tilt)))
                }
                // The knob, the last handle dragged: a stepper changes the wall at once, as it is squeezed. It bends as
                // far as the hand has moved to the right from where it took hold of it.
                _ => Some(laid_out(
                    start,
                    start.grid(),
                    start.scale(),
                    start.aspect,
                    bent(start, right),
                )),
            };
            if let Some(changed) = changed.filter(allowed) {
                // Reaching an end of the knob's or the hinge's range, or flat or upright, ticks, as a stop.
                let stopped = match drag.handle {
                    Handle::Bend => changed
                        .radius()
                        .is_none_or(|radius| bent(start, 1e3) == Some(radius)),
                    Handle::Tilt => {
                        let tilt = changed.tilt();
                        tilt == 0.0 || tilt.abs() >= layout::MOST_TILT
                    }
                    _ => false,
                };
                output.ticked |= stopped && !drag.stopped;
                drag.stopped = stopped;
                self.preview = Some(changed);
            }
        }
        // A wall carried follows the ray, facing the viewer, as far along it and as turned as the stick puts it.
        if let (Some(carry), Some(ray)) = (&mut self.carry, input.ray.filter(|_| input.active)) {
            output.ticked |= carry.manipulation.update(input.stick, input.dt);
            let at = ray.at(carry.manipulation.distance());
            // Its frame is tilted as it tilts, which carrying keeps, and its pose is upright.
            let tilt = Mat4::from_rotation_x(
                self.preview
                    .as_ref()
                    .map_or(0.0, |preview| preview.tilt().to_radians()),
            );
            // Squared up, it faces the participant, its tilt kept, from where it is carried.
            if input.straighten {
                let middle = carry.model.transform_point3(Vec3::ZERO);
                let (_, square, _) = facing(
                    middle,
                    Vec3::new(input.viewer.x, middle.y, input.viewer.z),
                    1.0,
                )
                .to_scale_rotation_translation();
                carry.facing = square * Quat::from_mat4(&tilt);
                carry.manipulation.straighten();
                output.ticked = true;
            }
            carry.model = Mat4::from_translation(at)
                * Mat4::from_rotation_y(carry.manipulation.yaw())
                * Mat4::from_quat(carry.facing)
                * Mat4::from_translation(-carry.grabbed);
            self.preview
                .iter_mut()
                .for_each(|preview| preview.pose = pose(carry.model * tilt.inverse()));
            // Pulled firmly towards the participant, it comes out of its group alone, and the rest stay.
            if !self.members.is_empty()
                && !self.detached
                && carry.start - carry.manipulation.distance() > DETACH
            {
                self.detached = true;
                output.ticked = true;
                // What is carried has changed, so a snap shown for the group is not held.
                self.snap = None;
                // Drawn alone from then on.
                self.preview.iter_mut().for_each(|preview| {
                    preview.group = None;
                    preview.joined.clear();
                });
            }
            let model = carry.model;
            self.moved = if self.detached {
                Vec::new()
            } else {
                self.members
                    .iter()
                    .map(|(wall, from)| {
                        let mut moved = wall.clone();
                        moved.pose = pose(model * *from);
                        moved
                    })
                    .collect()
            };
            // Brought near enough another wall, of those not carried, the snap of any wall carried that ranks first,
            // unless the trigger is held. The snap already shown is held a little wider, and ranks a little nearer, so
            // that it neither flickers at the edge of reach nor between two neighbours.
            let shown = self.snap.take();
            if input.rights.snap && !input.free {
                let carried: Vec<&Wall> = self.preview.iter().chain(&self.moved).collect();
                let detached = self.detached;
                let members = &self.members;
                // The wall pulled out of its group, as it stood in it.
                let origin = self
                    .preview
                    .as_ref()
                    .filter(|_| detached)
                    .and_then(|preview| input.projection.wall(&preview.id));
                self.snap = carried
                    .iter()
                    .flat_map(|wall| {
                        let away = Vec3::from_array(wall.pose.position).distance(input.viewer);
                        let shown = shown.as_ref();
                        walls
                            .iter()
                            .filter(|(other, _)| {
                                group::snaps_to(other)
                                    && !carried.iter().any(|carried| carried.id == other.id)
                            })
                            .filter_map(move |(other, _)| {
                                let held = shown
                                    .filter(|shown| shown.wall == wall.id && shown.to == other.id);
                                let mut leeway = group::Leeway::at(away);
                                if held.is_some() {
                                    leeway = leeway.held();
                                }
                                // Pulled out of its group, the walls it left stand where they stood.
                                let other = members
                                    .iter()
                                    .find(|(member, _)| detached && member.id == other.id)
                                    .map_or(*other, |(member, _)| member);
                                let snap = group::snap_within(wall, other, leeway)?;
                                // A snap that would put it back where it was pulled out from is offered only once it is
                                // pushed back nearly into the surface, so that the pull is not undone at once.
                                let back = origin.is_some_and(|origin: &Wall| {
                                    origin.id == wall.id
                                        && Vec3::from_array(origin.pose.position)
                                            .distance(Vec3::from_array(snap.pose.position))
                                            < RETURN
                                });
                                let mut snap = if back {
                                    group::snap_within(wall, other, leeway.within(PULLED_DEPTH))?
                                } else {
                                    snap
                                };
                                if held.is_some_and(|held| held.snap.edge == snap.edge) {
                                    snap.rank *= STAY;
                                }
                                Some(Snapping {
                                    wall: wall.id.clone(),
                                    to: other.id.clone(),
                                    snap,
                                })
                            })
                    })
                    .min_by(|a, b| a.snap.rank.total_cmp(&b.snap.rank));
            }
            output.ticked |= self.snap.as_ref().is_some_and(|snap| {
                shown.as_ref().is_none_or(|shown| {
                    (&shown.wall, &shown.to, shown.snap.edge)
                        != (&snap.wall, &snap.to, snap.snap.edge)
                })
            });
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
            // The version of the wall as its handle was taken hold of.
            let held = self.drag.take().map(|drag| drag.start.version);
            let dragged = held.is_some();
            let moved = std::mem::take(&mut self.moved);
            let detached = std::mem::take(&mut self.detached);
            self.members.clear();
            let snapped = self.snap.take().filter(|_| carried);
            if let Some(snapping) = snapped
                && let Some(after) = self.preview.take()
            {
                // Snapped, it is sent beside the other wall, and shown there with the walls carried with it.
                let from = std::iter::once(&after)
                    .chain(&moved)
                    .find(|wall| wall.id == snapping.wall)
                    .map_or(after.pose, |wall| wall.pose);
                let to = snapping.snap.pose;
                let placed = |wall: &Wall| {
                    let mut placed = curved_at(wall, snapping.snap.radius);
                    placed.pose = group::carried_with(&from, &to, &wall.pose);
                    placed
                };
                output.change = Some(Change {
                    wall: snapping.wall.clone(),
                    grid: None,
                    scale: None,
                    aspect: None,
                    shape: None,
                    pose: None,
                    tilt: None,
                    join: Some(Joining::Attach {
                        attach: Attach::new(snapping.to, snapping.snap.edge, snapping.snap.align),
                        alone: detached,
                    }),
                    version: None,
                });
                self.pending = Some((placed(&after), input.now));
                self.moved_pending = moved.iter().map(placed).collect();
            } else if let (Some(after), Some(before), true) =
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
                    tilt: ((after.tilt() - before.tilt()).abs() > 0.05).then(|| after.tilt()),
                    pose: (carried
                        && (Vec3::from_array(after.pose.position)
                            .distance(Vec3::from_array(before.pose.position))
                            > 1e-3
                            || Quat::from_array(after.pose.orientation)
                                .angle_between(Quat::from_array(before.pose.orientation))
                                > 1e-3))
                        .then_some(after.pose),
                    join: None,
                    // Dragged by a handle, it is changed from how it was taken hold of; carried, it is the
                    // participant's own wherever it was.
                    version: held,
                };
                // Pulled out of its group, it leaves it where it was let go; otherwise its group moves with it.
                let change = if detached {
                    Change {
                        pose: Some(after.pose),
                        join: Some(Joining::Leave),
                        ..change
                    }
                } else {
                    change
                };
                if change.grid.is_some()
                    || change.scale.is_some()
                    || change.aspect.is_some()
                    || change.shape.is_some()
                    || change.pose.is_some()
                    || change.tilt.is_some()
                {
                    output.change = Some(change);
                    self.pending = Some((after, input.now));
                    self.moved_pending = moved;
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
                    .or_else(|| model(&wall.frame()))
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
            let all: Vec<&Wall> = shown.iter().collect();
            let keep = self.drag.as_ref().map(|drag| drag.handle);
            let controls = LockControls::of(wall, input.rights);
            let (points, _) =
                handle_points_among_with(wall, *frame, input.viewer, &all, keep, controls);
            let at = points
                .iter()
                .find(|(shown, _)| *shown == handle)
                .map_or(frame.transform_point3(Vec3::ZERO), |(_, at)| *at);
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
                .chain(bar_met.as_ref().map(|(_, along)| *along))
                .min_by(f32::total_cmp)
                .map(|along| crate::pointing::End::new(along, ray.at(along)));
        }
        output.sheets = self.sheets(walls, &input);
        output
    }

    /// Each wall's bar, for a participant who may adjust walls, highlighted while the ray meets it or its wall is
    /// carried, with what the grip does there while it is not taken hold of; and the handles of the wall taken hold of,
    /// facing the viewer, and the hint above it, as they are drawn.
    fn sheets(&self, walls: &[(&Wall, Mat4)], input: &Input<'_>) -> Vec<Sheet> {
        let mut sheets = Vec::new();
        let all: Vec<&Wall> = walls.iter().map(|(wall, _)| *wall).collect();
        if input.rights.adjust {
            for (wall, frame) in walls {
                let carried = self.carrying() == Some(wall.id.as_str());
                let (wall, frame) = match (&self.carry, &self.preview) {
                    (Some(carry), Some(preview)) if carried => (preview, carry.model),
                    _ => (*wall, *frame),
                };
                // A group shows one bar, above its top left wall.
                let Some(bar) = group_bar(wall, frame, input.viewer, &all) else {
                    continue;
                };
                let on = self.bar.as_deref() == Some(wall.id.as_str());
                sheets.push(Sheet {
                    model: Some(bar.model),
                    canvas: Some(std::sync::Arc::new(bar_face(&bar, on || carried))),
                });
                if on && self.editing() != Some(wall.id.as_str()) {
                    let up = frame.transform_vector3(Vec3::Y).normalize_or(Vec3::Y);
                    let side = if wall.kind == WallKind::Tomori {
                        -1.0
                    } else {
                        1.0
                    };
                    let edge = bar.middle + up * side * bar.size * HANDLE_REACH;
                    let mut sheet = hint_above(
                        bar_label_for(input.controls, wall, input.rights),
                        edge,
                        input.viewer,
                    );
                    // Below a bar below its wall, clear of it.
                    if side < 0.0
                        && let (Some(model), Some(canvas)) = (&mut sheet.model, &sheet.canvas)
                    {
                        *model = Mat4::from_translation(-Vec3::Y * canvas.size().1) * *model;
                    }
                    sheets.push(sheet);
                }
            }
        }
        let Some((wall, frame)) = self
            .wall
            .as_ref()
            .and_then(|id| walls.iter().find(|(wall, _)| wall.id == *id))
            .map(|(wall, frame)| (*wall, *frame))
        else {
            return sheets;
        };
        let preview = self
            .preview
            .as_ref()
            .filter(|preview| preview.id == wall.id);
        let wall = preview.unwrap_or(wall);
        let frame = match &self.carry {
            Some(carry) => carry.model,
            None => model(&wall.frame()).unwrap_or(frame),
        };
        // Where the walls carried would snap: each slot of the one that snaps outlined where it would stand.
        let snapped: Vec<Wall> = self
            .snap
            .iter()
            .flat_map(|snapping| {
                self.preview
                    .iter()
                    .chain(&self.moved)
                    .filter(move |shown| shown.id == snapping.wall)
                    .map(move |shown| {
                        let mut snapped = curved_at(shown, snapping.snap.radius);
                        snapped.pose = snapping.snap.pose;
                        snapped
                    })
            })
            .collect();
        for snapped in &snapped {
            let cells = Cells::of(snapped);
            let slots = model(&snapped.pose)
                .into_iter()
                .flat_map(|at| super::slot_frames(snapped, at));
            sheets.extend(slots.map(|slot| Sheet {
                canvas: Some(std::sync::Arc::new(ghost(cells))),
                model: Some(slot * Mat4::from_translation(Vec3::Z * LIFT)),
            }));
        }
        let keep = self.drag.as_ref().map(|drag| drag.handle);
        let controls = LockControls::of(wall, input.rights);
        let (points, size) =
            handle_points_among_with(wall, frame, input.viewer, &all, keep, controls);
        if self.carry.is_none() {
            for (handle, at) in &points {
                sheets.push(Sheet {
                    model: Some(facing(*at, input.viewer, 1.0)),
                    canvas: Some(std::sync::Arc::new(handle_face(
                        *handle,
                        size,
                        self.hovered == Some(*handle),
                        wall.is_locked(),
                    ))),
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
            None => hint_for(
                input.controls,
                wall,
                self.carry.is_some(),
                input.rights,
                self.confirm.map(|_| held),
                Grouping {
                    grouped: !self.members.is_empty() && !self.detached,
                    snapping: self.snap.is_some(),
                },
            ),
        };
        // Above the wall and its handles, clear of the status pill, which stands low in the view, facing the viewer at the
        // angle the hint beside a panel carried is drawn.
        let up = frame.transform_vector3(Vec3::Y).normalize_or(Vec3::Y);
        let (knob, _) = handle_point_among(wall, frame, Handle::Bend, input.viewer, &all, keep);
        // In a group, above its other walls' slots and its bar too, so that it covers nothing the group shows.
        let slots = super::boards::members_slots(wall, &all)
            .into_iter()
            .map(|slots| frame.transform_point3(Vec3::new(0.0, slots.top, 0.0)));
        let bars = walls
            .iter()
            .filter(|(other, _)| other.group.is_some() && other.group == wall.group)
            .filter_map(|(other, at)| group_bar(other, *at, input.viewer, &all))
            .map(|bar| bar.middle + up * bar.size * BAR_THICKNESS / 2.0);
        let top = points
            .iter()
            .map(|(_, at)| *at)
            .chain(slots)
            .chain(bars)
            .max_by(|a, b| a.dot(up).total_cmp(&b.dot(up)))
            .unwrap_or(knob);
        let above = knob + up * ((top - knob).dot(up) + size);
        sheets.push(hint_above(hint, above, input.viewer));
        sheets
    }
}

/// `wall`, curved at `radius` where both are curved, as a wall snapped to a curved one is, continuing its arc, and
/// upright, as every wall of a group stands.
fn curved_at(wall: &Wall, radius: Option<f32>) -> Wall {
    let upright = wall.clone().with_tilt(None);
    match radius {
        Some(radius) if wall.radius().is_some() => upright.with_shape(WallShape::Arc { radius }),
        _ => upright,
    }
}

/// A bar's face, as long and thick as `bar`, subtle, and bright while `highlighted`.
fn bar_face(bar: &Bar, highlighted: bool) -> Canvas {
    // Eight font pixels thick, and as many long as it is.
    const THICK: usize = 8;
    let pixel = (BAR_THICKNESS * bar.size / THICK as f32).max(1e-5);
    let mut canvas = Canvas::new(0, pixel);
    canvas.padding = (
        ((bar.half.length() / pixel).round() as usize).max(THICK),
        THICK / 2,
    );
    canvas.background = if highlighted {
        HIGHLIGHT
    } else {
        [0.62, 0.68, 0.78, 0.42]
    };
    canvas
}

/// What the grip does on a bar of `wall`, as `controls` name it: takes hold of the wall to edit it, carrying one someone
/// added.
pub fn bar_label(controls: Controls, wall: &Wall) -> Canvas {
    bar_label_for(
        controls,
        wall,
        Rights {
            adjust: true,
            ..Rights::default()
        },
    )
}

/// What the grip does on a bar of `wall` for a participant with `rights`: takes hold of it to edit it, carrying one it
/// carries; or, where it is locked, takes hold of it to show its padlock, which an owner unlocks it with.
pub fn bar_label_for(controls: Controls, wall: &Wall, rights: Rights) -> Canvas {
    let grip = if controls == Controls::Keyboard {
        "G"
    } else {
        "Grip"
    };
    let action = if rights.locks && wall.is_locked() {
        if rights.lock {
            "locked: take hold to unlock it"
        } else {
            "locked by an owner"
        }
    } else if carried(wall, rights) {
        "carry and edit this wall"
    } else {
        "edit this wall"
    };
    let columns = crate::ui_text::width(grip) + 2 + crate::ui_text::width(action);
    let mut canvas = Canvas::new(columns, HINT_ANGLE);
    canvas.padding = (5, 4);
    canvas.background = [0.0, 0.0, 0.0, 0.82];
    canvas.border = Some(FRAME);
    canvas.rows = vec![Row::new(cut(
        &[
            crate::ui_text::Span::new(
                crate::ui_text::pad(grip, crate::ui_text::width(grip) + 2),
                Style::bold(BRIGHT),
            ),
            crate::ui_text::Span::new(action, Style::plain(TEXT)),
        ],
        columns,
    ))];
    canvas
}

/// What a handle dragged makes of `wall`, as it would be let go now: for a corner, its slots' size and shape, such as
/// `60 × 107 cm · 9:16`; for the knob, its curve, such as `◠ radius 2.4 m`, or `flat`; for the hinge, its tilt, such
/// as `tilted 12° down`, or `upright`; for an edge, its grid, such as `4 × 2 slots`.
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
        Handle::Tilt => match wall.tilt() {
            tilt if tilt > 0.0 => format!("tilted {tilt:.0}° down"),
            tilt if tilt < 0.0 => format!("tilted {:.0}° up", -tilt),
            _ => "upright".to_owned(),
        },
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
        canvas: Some(std::sync::Arc::new(canvas)),
    }
}

/// Where a wall carried stands with others: in a group, which it carries, and snapping beside another wall.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Grouping {
    pub grouped: bool,
    pub snapping: bool,
}

/// The hint beside a wall taken hold of, as [`hint_with`] says for a wall alone that would not snap.
pub fn hint(
    controls: Controls,
    wall: &Wall,
    carried: bool,
    removes: bool,
    confirm: Option<usize>,
) -> Canvas {
    hint_with(
        controls,
        wall,
        carried,
        removes,
        confirm,
        Grouping::default(),
    )
}

/// The hint beside a wall taken hold of: what each control does to it, as `controls` name them, for `wall`, `carried`
/// or not, saying B removes it where the participant `removes` walls, and, once asked to with panels on it, `confirm`,
/// how many of them would float; and, carried, how to pull it out of its group, `grouped`, and that letting go snaps it
/// beside another, `snapping`.
pub fn hint_with(
    controls: Controls,
    wall: &Wall,
    carried: bool,
    removes: bool,
    confirm: Option<usize>,
    grouping: Grouping,
) -> Canvas {
    let rights = Rights {
        adjust: true,
        remove: removes,
        ..Rights::default()
    };
    hint_for(controls, wall, carried, rights, confirm, grouping)
}

/// The hint beside a wall taken hold of, as [`hint_with`] says it, for a participant with `rights`: B removes one
/// someone added where they remove walls, and the bar carries one they carry. Where the server locks walls, a locked
/// wall's says only that it is locked and, for an owner, that the grip on the padlock unlocks it; and an unlocked wall's
/// says too, for an owner, that it locks it, and, for an environment's own, that the grip on `↺` puts it back, locked.
pub fn hint_for(
    controls: Controls,
    wall: &Wall,
    carried: bool,
    rights: Rights,
    confirm: Option<usize>,
    grouping: Grouping,
) -> Canvas {
    let keyboard = controls == Controls::Keyboard;
    let grip = if keyboard { "G" } else { "Grip" };
    let removes = rights.remove && wall.kind == WallKind::Added;
    let lock = LockControls::of(wall, rights);
    let mut rows: Vec<(String, String, bool)> = Vec::new();
    let done = (
        (if keyboard { "Backspace" } else { "B" }).to_owned(),
        "done".to_owned(),
        false,
    );
    if lock.padlock && wall.is_locked() {
        rows.push(if rights.lock {
            (
                format!("{grip} lock"),
                "unlock it to change it".to_owned(),
                false,
            )
        } else {
            (
                "Locked".to_owned(),
                "an owner can unlock it".to_owned(),
                false,
            )
        });
        rows.push(done.clone());
    } else if carried {
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
        if grouping.grouped {
            rows.push((
                (if keyboard {
                    "PgDn far"
                } else {
                    "Stick ↓ far"
                })
                .to_owned(),
                "take it out alone".to_owned(),
                false,
            ));
        }
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
        if grouping.snapping {
            rows.push((
                (if keyboard {
                    "Hold click"
                } else {
                    "Hold trigger"
                })
                .to_owned(),
                "don't snap".to_owned(),
                false,
            ));
        }
        rows.push((
            format!("Let go {}", if keyboard { "G" } else { "" })
                .trim()
                .to_owned(),
            (if grouping.snapping {
                "snap it there"
            } else {
                "leave it here"
            })
            .to_owned(),
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
        // The knob moved across bends, and the hinge moved up or down tilts.
        rows.push((format!("{grip} knob"), "↔ bend".to_owned(), false));
        rows.push((format!("{grip} hinge"), "↕ tilt".to_owned(), false));
        if wall.kind == WallKind::Added || (rights.locks && self::carried(wall, rights)) {
            rows.push((format!("{grip} bar"), "carry it".to_owned(), false));
        }
        if lock.padlock && rights.lock {
            rows.push((format!("{grip} lock"), "lock it as it is".to_owned(), false));
        }
        if lock.reset {
            rows.push((format!("{grip} ↺"), "put back, locked".to_owned(), false));
        }
        rows.push(done);
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
mod group_controls_tests;
#[cfg(test)]
mod snapping_tests;
#[cfg(test)]
mod tests;
