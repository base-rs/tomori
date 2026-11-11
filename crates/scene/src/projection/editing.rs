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
    locomotion::Locomotion,
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

/// The step the text handle scales a wall's content by, as a proportion of the text its slots give it: a tenth, so that
/// its readout is a round number and a face is laid out again only as the step changes.
pub const CONTENT_STEP: f32 = 0.1;

/// How large a handle is on a wall of scale 1, in metres: about 1.7° from 2.5 m, as near as a standard wall is read
/// from. A wall's handles are as much larger as its slots are, since a wall is read from as much further away, so that a
/// large wall far away shows its handles as a standard one near does; see [`handle_size`].
pub const HANDLE_SIZE: f32 = 0.075;

/// The least and the most a handle is, in metres, whatever its wall's scale: large enough to aim at on the smallest wall
/// and no slab on the largest.
pub const HANDLE_SIZES: (f32, f32) = (0.045, 0.3);

/// How far apart the middles of neighbouring handles stand, in handles: a gap of most of a handle between them.
pub const SPACING: f32 = 1.8;

/// How near a handle's middle, across or up its face, the ray meets it, as a proportion of its size: never as far as
/// halfway to its neighbour's, so that the ray meets at most one of two neighbours.
const HANDLE_REACH: f32 = 0.8;

/// How near the viewer a locked wall's padlock stands for an owner to see it, in metres: near enough to walk up to, so
/// that walls far away show nothing.
pub const PADLOCK_NEAR: f32 = 3.0;

/// Whether a surface among `surfaces` stands between `from` and `to`, crossing the line between them, as the floor of a
/// deck stands between a participant on it and a wall beneath it.
fn hidden(surfaces: Option<&Locomotion>, from: Vec3, to: Vec3) -> bool {
    surfaces
        .zip(Ray::new(from, to - from))
        .is_some_and(|(surfaces, ray)| surfaces.surface_crossed(&ray, from.distance(to) - 0.01))
}

/// Whether a surface among `surfaces` crosses `ray` before `along` it: what the ray meets there is beyond a floor.
pub(super) fn beyond(surfaces: Option<&Locomotion>, ray: &Ray, along: f32) -> bool {
    surfaces.is_some_and(|surfaces| surfaces.surface_crossed(ray, along - 0.01))
}

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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Axis {
    Columns,
    Rows,
}

/// A handle on a wall taken hold of.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
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
    /// The text handle, `aA`, on its bar's row, to the bar's left:
    /// moved to the right, as the participant sees it, from where it was taken hold of, it draws the content of the
    /// panels in its slots larger, and back to the left, smaller, so that each slot shows more and longer lines, like a
    /// slider, its slots as they were; moving up or down does nothing.
    Text,
    /// The padlock a row beyond its top left corner, open, on a wall taken hold of: squeezed by an owner, it locks the
    /// wall, for everyone. A locked wall, never taken hold of, shows it alone and closed to an owner near it, whose
    /// squeeze unlocks it.
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
    /// The server locks walls: each says whether it is locked, a locked wall is never taken hold of and shows only its
    /// padlock, to an owner near it, and an environment's own wall, unlocked, is carried as one someone added is.
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

/// A locked wall's padlock as last laid out, with what it was laid out from: the wall, where it stood and the other
/// walls of its group, so that it is laid out again only when one of them changes.
#[derive(Clone, Debug)]
struct PadlockLayout {
    wall: Wall,
    frame: Mat4,
    members: Vec<Wall>,
    models: Vec<Mat4>,
    size: f32,
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
    /// How large its panels' content is drawn within its slots, as a proportion of the text they give it, `1` for as
    /// large.
    pub content_scale: Option<f32>,
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
    /// The locked wall whose padlock the ray meets, by identity.
    padlock: Option<String>,
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
    /// Each wall's bar's face last drawn, by its wall's identity, with what it was drawn from, handed over again while
    /// that is the same, so that it is not drawn again and its mesh is kept.
    bars: std::collections::HashMap<String, (BarFace, std::sync::Arc<crate::canvas::Canvas>)>,
    /// Each locked wall's padlock as last laid out, by its wall's identity, for an owner.
    padlock_layouts: std::collections::HashMap<String, PadlockLayout>,
    /// Each handle's face drawn, by its handle, whether it is highlighted and whether its wall is locked, a handle
    /// across, handed over again whichever wall shows it, so that it is not drawn again and its mesh is kept.
    faces: std::collections::HashMap<(Handle, bool, bool), std::sync::Arc<crate::canvas::Canvas>>,
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
    /// The environment's declared surfaces, its floors and decks, which hide a padlock beyond them from the viewer and
    /// stop the ray before a padlock, a handle, a bar or a wall beyond them; `None` where nothing is declared.
    pub surfaces: Option<&'a Locomotion>,
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
fn handle_models(
    wall: &Wall,
    frame: Mat4,
    walls: &[&Wall],
    keep: Option<Handle>,
    controls: LockControls,
) -> (Vec<(Handle, Mat4)>, f32) {
    let mut around = Around::of(wall, frame, walls, keep);
    around.controls = controls;
    let (spots, size) = lay_out(wall, frame, &around);
    let models = spots
        .into_iter()
        .map(|(handle, spot)| (handle, spot.model(frame)))
        .collect();
    (models, size)
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
    (!wall.is_locked() && super::boards::bears_bar(wall, walls)).then(|| {
        placed_bar(wall, frame, &super::boards::members_slots(wall, walls)).at(wall, frame)
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
}

impl Around {
    fn of(wall: &Wall, frame: Mat4, walls: &[&Wall], keep: Option<Handle>) -> Self {
        let slots = super::boards::members_slots(wall, walls);
        let bar = if slots.is_empty() {
            None
        } else if super::boards::bears_bar(wall, walls) {
            Some(placed_bar(wall, frame, &slots))
        } else {
            super::boards::members(wall, walls)
                .into_iter()
                .find(|other| super::boards::bears_bar(other, walls))
                .map(|bearer| {
                    // Standing as it stands from this wall.
                    let at = frame
                        * model(&wall.pose).unwrap_or_default().inverse()
                        * model(&bearer.pose).unwrap_or_default();
                    let placed =
                        placed_bar(bearer, at, &super::boards::members_slots(bearer, walls));
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
            joined: super::boards::joined_edges(wall, walls),
            keep,
            bar,
            controls: LockControls::default(),
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
const BAR_THICKNESS: f32 = 0.45;

/// How long a bar is, in handles, at least and at most, about a third of the slots it stands below between.
const BAR_LEAST: f32 = 3.0;
const BAR_MOST: f32 = 5.0;

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
            Some(_) => super::ARC_LABEL_DROP + super::arc_label(wall).size().1 / 2.0,
            None => 0.0,
        };
        let half = bar_half(width, size);
        let top = (height / 2.0 + out).max(pitch);
        let bottom = -(height / 2.0 + label + out).max(pitch);
        let low = frame
            .transform_point3(along(wall, 0.0).0 + Vec3::Y * bottom)
            .y
            - size / 2.0;
        let raised = low < tomori_protocol::entourage::FLOOR + FLOOR_CLEARANCE;
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
    fn model(self, frame: Mat4) -> Mat4 {
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
fn along(wall: &Wall, x: f32) -> (Vec3, Vec3) {
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
                (towards * (inward - side), row)
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
                >= tomori_protocol::entourage::FLOOR + FLOOR_CLEARANCE;
            (dragged || (clear && above)).then_some((handle, spot))
        })
        .collect();
    (spots, size)
}

/// The handle of `models`, each `size` across, that `ray` meets, and how far along it: where it crosses a handle's plane
/// in front of it, within [`HANDLE_REACH`] of a handle's size of its middle, across or up its face, the nearest along
/// it. Neighbours' reaches leave a gap between them in their wall's plane, so that the ray meets at most one of two.
fn nearest_handle(models: &[(Handle, Mat4)], size: f32, ray: &Ray) -> Option<(Handle, f32)> {
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
        Handle::Text => text_shapes(rule),
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

/// The text handle's face, in font pixels from its cell's middle, drawn with `rule`: a small `A` and a large one beside
/// it, `aA`, as text sizes are shown, so that it reads as the size of the text, centred on its handle.
fn text_shapes(rule: impl Fn(Vec2, Vec2) -> crate::shapes::Shape) -> Vec<crate::shapes::Shape> {
    // Each `A` from its apex to its feet, with its bar across at `bar` of the way down.
    let letter = |apex: Vec2, left: Vec2, right: Vec2, bar: f32| {
        [
            rule(apex, left),
            rule(apex, right),
            rule(apex.lerp(left, bar), apex.lerp(right, bar)),
        ]
    };
    letter(
        Vec2::new(-3.75, 0.5),
        Vec2::new(-5.5, 4.5),
        Vec2::new(-2.0, 4.5),
        0.62,
    )
    .into_iter()
    .chain(letter(
        Vec2::new(2.25, -4.5),
        Vec2::new(-1.0, 4.5),
        Vec2::new(5.5, 4.5),
        0.66,
    ))
    .collect()
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

/// How large `wall`, as it was when its text handle was taken hold of, draws its panels' content within its slots when
/// the hand has moved `moved` metres to the right, as the participant sees it, from where it took hold of it: as it
/// was, larger as the hand moves right and smaller as it moves left, by as many times over [`TRAVEL`] as the whole of
/// [`layout::CONTENT_SCALES`] spans, alike either way, so that one movement of 25 cm takes it from half to twice, in
/// steps of [`CONTENT_STEP`], never below its [`Wall::least_content_scale`] nor beyond the most. Within [`DEAD_ZONE`]
/// of where it was taken, it is as it was.
pub fn content_scaled(wall: &Wall, moved: f32) -> f32 {
    let start = wall.content_scale();
    let moved = moved_by(moved);
    if moved == 0.0 {
        return start;
    }
    let (least, most) = (wall.least_content_scale(), layout::CONTENT_SCALES.1);
    let span = layout::CONTENT_SCALES.1 / layout::CONTENT_SCALES.0;
    let scaled = start * span.powf(moved);
    let steps = (scaled / CONTENT_STEP).round();
    (steps / (1.0 / CONTENT_STEP)).clamp(least, most)
}

/// `wall` with its panels' content drawn `scale` times the size its slots give it, saying none where that is 1.
fn content_at(wall: &Wall, scale: f32) -> Wall {
    wall.clone()
        .with_content_scale(((scale - 1.0).abs() > 1e-4).then_some(scale))
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

    /// Whether a wall is carried or a handle dragged, so that the hand doing it keeps the editor until it lets go.
    pub fn busy(&self) -> bool {
        self.drag.is_some() || self.carry.is_some()
    }

    /// What `ray`, the ray of a hand the editor does not follow, meets of `walls`, with `rights`: how
    /// far along it a handle of the wall taken hold of, or a bar, is, which it meets before a panel behind it, and where
    /// it ends on the nearest of those and the walls it may take hold of, as [`Self::update`] says of the hand it
    /// follows. It changes nothing, and takes no squeeze.
    pub fn probe(
        &self,
        ray: &Ray,
        walls: &[(&Wall, Mat4)],
        rights: Rights,
    ) -> (Option<f32>, Option<crate::pointing::End>) {
        if !rights.adjust {
            return (None, None);
        }
        let all: Vec<&Wall> = walls.iter().map(|(wall, _)| *wall).collect();
        let handle = self
            .wall
            .as_ref()
            .filter(|_| !self.busy())
            .and_then(|id| walls.iter().find(|(wall, _)| wall.id == *id))
            .and_then(|(wall, frame)| {
                let controls = LockControls::of(wall, rights);
                let (models, size) = handle_models(wall, *frame, &all, None, controls);
                nearest_handle(&models, size, ray)
            })
            .map(|(_, along)| along);
        let bar = walls
            .iter()
            .filter_map(|(wall, frame)| group_bar(wall, *frame, &all).and_then(|bar| bar.hit(ray)))
            .min_by(f32::total_cmp);
        let reach = handle.or(bar);
        let along = walls
            .iter()
            .filter_map(|(wall, frame)| super::wall_hit(wall, *frame, ray))
            .chain(reach)
            .min_by(f32::total_cmp);
        (
            reach,
            along.map(|along| crate::pointing::End::new(along, ray.at(along))),
        )
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
        self.padlock = None;
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

    /// The padlocks of the locked walls among `walls`, as drawn, that a participant with `rights` sees, each with its
    /// wall, as its face's model and size: for an owner, where the server locks walls, each standing within
    /// [`PADLOCK_NEAR`] of `viewer`, where the padlock of the wall taken hold of stands, with none of `surfaces` between
    /// `viewer` and the padlock or the middle of its wall, so that a wall beneath a deck shows nothing to an owner on
    /// it, however near. Anyone else, and an owner further away, sees none, as a locked wall shows nothing else. Each is
    /// laid out once and kept until its wall, where it stands or its group changes.
    fn padlocks<'a>(
        &mut self,
        walls: &[(&'a Wall, Mat4)],
        viewer: Vec3,
        surfaces: Option<&Locomotion>,
        rights: Rights,
    ) -> Vec<(&'a Wall, Mat4, f32)> {
        if !(rights.adjust && rights.locks && rights.lock) {
            return Vec::new();
        }
        let all: Vec<&Wall> = walls.iter().map(|(wall, _)| *wall).collect();
        self.padlock_layouts
            .retain(|id, _| all.iter().any(|wall| wall.is_locked() && wall.id == *id));
        let controls = LockControls {
            padlock: true,
            reset: false,
        };
        let mut padlocks = Vec::new();
        for (wall, frame) in walls.iter().filter(|(wall, _)| wall.is_locked()) {
            let members = super::boards::members(wall, &all);
            let kept = self.padlock_layouts.get(&wall.id).filter(|kept| {
                kept.wall == **wall
                    && kept.frame == *frame
                    && kept.members.iter().eq(members.iter().copied())
            });
            let (models, size) = match kept {
                Some(kept) => (kept.models.clone(), kept.size),
                None => {
                    let (models, size) = handle_models(wall, *frame, &all, None, controls);
                    let models: Vec<Mat4> = models.into_iter().map(|(_, model)| model).collect();
                    self.padlock_layouts.insert(
                        wall.id.clone(),
                        PadlockLayout {
                            wall: (*wall).clone(),
                            frame: *frame,
                            members: members.into_iter().cloned().collect(),
                            models: models.clone(),
                            size,
                        },
                    );
                    (models, size)
                }
            };
            let middle = frame.w_axis.truncate();
            padlocks.extend(
                models
                    .into_iter()
                    .filter(|model| {
                        let padlock = model.w_axis.truncate();
                        padlock.distance(viewer) <= PADLOCK_NEAR
                            && !hidden(surfaces, viewer, padlock)
                            && !hidden(surfaces, viewer, middle)
                    })
                    .map(|model| (*wall, model, size)),
            );
        }
        padlocks
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
                    && (wall.content_scale() - sent.content_scale()).abs() < 1e-3
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
        // Locked, by an owner here or elsewhere, as the server or what was last sent has it, a wall is let go of, as a
        // locked wall is never held: only its padlock shows, to an owner near it.
        if self
            .wall
            .as_ref()
            .and_then(|id| {
                self.pending
                    .as_ref()
                    .map(|(sent, _)| sent)
                    .filter(|sent| sent.id == *id)
                    .or_else(|| input.projection.wall(id))
            })
            .is_some_and(Wall::is_locked)
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
        // The handle the ray meets, unless one is dragged: where it crosses a handle's face, within its reach.
        let mut met: Option<(Handle, f32)> = None;
        let all: Vec<&Wall> = walls.iter().map(|(wall, _)| *wall).collect();
        if let (Some((wall, frame)), Some(ray), None, None) =
            (&editing, ray, &self.drag, &self.carry)
        {
            let controls = LockControls::of(wall, input.rights);
            let (models, size) = handle_models(wall, *frame, &all, None, controls);
            met = nearest_handle(&models, size, &ray)
                .filter(|(_, along)| !beyond(input.surfaces, &ray, *along));
        }
        let hovered = self
            .drag
            .as_ref()
            .map(|drag| drag.handle)
            .or(met.map(|(handle, _)| handle));
        // The padlock of a locked wall the ray meets, where it meets no handle, nearest along it, for an owner near it.
        let padlocks = self.padlocks(walls, input.viewer, input.surfaces, input.rights);
        let padlock_met = ray
            .filter(|_| met.is_none() && self.drag.is_none() && self.carry.is_none())
            .and_then(|ray| {
                padlocks
                    .iter()
                    .filter_map(|(wall, model, size)| {
                        nearest_handle(&[(Handle::Lock, *model)], *size, &ray)
                            .map(|(_, along)| (*wall, along))
                    })
                    .min_by(|a, b| a.1.total_cmp(&b.1))
                    .filter(|(_, along)| !beyond(input.surfaces, &ray, *along))
            });
        // The bar the ray meets, where it meets no handle, nearest along it, for a participant who may adjust walls.
        let bar_met = ray
            .filter(|_| {
                input.rights.adjust
                    && met.is_none()
                    && padlock_met.is_none()
                    && self.drag.is_none()
                    && self.carry.is_none()
            })
            .and_then(|ray| {
                walls
                    .iter()
                    .filter_map(|(wall, frame)| {
                        group_bar(wall, *frame, &all)?
                            .hit(&ray)
                            .map(|along| (wall.id.clone(), along))
                    })
                    .min_by(|a, b| a.1.total_cmp(&b.1))
                    .filter(|(_, along)| !beyond(input.surfaces, &ray, *along))
            });
        // Where the ray meets a locked wall nearer than anything else it would take, and no handle, padlock or bar: it
        // says the wall is locked and, to an owner too far to see its padlock, to come closer, so that it is clear why the
        // grip takes nothing there; not where a floor or deck hides its padlock or its wall, as coming closer would not
        // show it.
        let locked_met = ray
            .filter(|_| {
                input.rights.adjust
                    && met.is_none()
                    && padlock_met.is_none()
                    && bar_met.is_none()
                    && self.drag.is_none()
                    && self.carry.is_none()
            })
            .and_then(|ray| {
                walls
                    .iter()
                    .filter_map(|(wall, frame)| {
                        super::wall_hit(wall, *frame, &ray).map(|along| (*wall, along))
                    })
                    .min_by(|a, b| a.1.total_cmp(&b.1))
                    .filter(|(wall, along)| {
                        wall.is_locked()
                            && *along < input.nearer
                            && *along < input.panel
                            && !beyond(input.surfaces, &ray, *along)
                            && !padlocks.iter().any(|(near, ..)| near.id == wall.id)
                    })
                    .map(|(wall, along)| {
                        let middle = walls
                            .iter()
                            .find(|(other, _)| other.id == wall.id)
                            .map(|(_, frame)| frame.w_axis.truncate());
                        let clear = middle
                            .into_iter()
                            .chain(self.padlock_layouts.get(&wall.id).into_iter().flat_map(
                                |kept| kept.models.iter().map(|model| model.w_axis.truncate()),
                            ))
                            .all(|point| !hidden(input.surfaces, input.viewer, point));
                        (ray.at(along), input.rights.lock && clear)
                    })
            });
        let on_bar = bar_met.as_ref().map(|(wall, _)| wall.clone());
        let on_padlock = padlock_met.map(|(wall, _)| wall.id.clone());
        output.ticked = (hovered.is_some() && hovered != self.hovered)
            || (on_bar.is_some() && on_bar != self.bar)
            || (on_padlock.is_some() && on_padlock != self.padlock);
        self.hovered = hovered;
        self.bar = on_bar;
        self.padlock = on_padlock;
        // A handle, a padlock or a bar the ray meets is met before a panel behind it.
        output.reach = met
            .map(|(_, along)| along)
            .or(padlock_met.map(|(_, along)| along))
            .or(bar_met.as_ref().map(|(_, along)| *along));
        // A squeeze takes the handle the ray meets, or, failing anything nearer, the wall it meets.
        if input.pressed && input.active && input.rights.adjust {
            if let (Some((handle, along)), Some((wall, frame)), Some(ray)) = (met, &editing, ray)
                && along < input.nearer
            {
                output.took = true;
                match handle {
                    // The padlock locks the wall, for an owner, shown so at once, and lets go of it, as a locked wall is
                    // never held; for anyone else it only says it is unlocked. Putting it back moves it where the
                    // server says.
                    Handle::Lock | Handle::Reset => {
                        if input.rights.lock {
                            let reset = handle == Handle::Reset;
                            output.lock = Some(LockChange {
                                wall: wall.id.clone(),
                                locked: true,
                                reset,
                            });
                            self.stop();
                            self.pending =
                                (!reset).then(|| (wall.clone().with_locked(true), input.now));
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
                                content_scale: None,
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
            } else if let Some((wall, along)) = padlock_met
                && along < input.nearer
            {
                // A locked wall's padlock, squeezed by an owner, unlocks it, for everyone, and takes hold of it, shown
                // unlocked at once with every handle.
                output.took = true;
                output.ticked = true;
                output.lock = Some(LockChange {
                    wall: wall.id.clone(),
                    locked: false,
                    reset: false,
                });
                self.stop();
                self.wall = Some(wall.id.clone());
                self.pending = Some((wall.clone().with_locked(false), input.now));
            } else if let Some(ray) = ray.filter(|_| self.drag.is_none() && self.carry.is_none()) {
                // The wall's board, an empty slot or its bar nearer than anything else: it is taken hold of, and one
                // someone added is carried. A locked wall is taken hold of by nothing, though it stands in the way.
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
                    .min_by(|a, b| a.2.total_cmp(&b.2))
                    .filter(|(wall, _, along)| {
                        !wall.is_locked() && !beyond(input.surfaces, &ray, *along)
                    });
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
                // An edge follows the hand from where it took hold, its handle standing beyond the edge it moves.
                Handle::Columns => on_plane(frame, &ray).map(|at| {
                    let beyond = drag.grab.x.abs() - corners(start)[1].x;
                    let at = Vec3::new(at.x.abs() - beyond, at.y, at.z);
                    with_grid(start, nearest_grid(start, Axis::Columns, at))
                }),
                Handle::Rows => on_plane(frame, &ray).map(|at| {
                    let beyond = -drag.grab.y.abs() - corners(start)[3].y;
                    let at = Vec3::new(at.x, -at.y.abs() - beyond, at.z);
                    with_grid(start, nearest_grid(start, Axis::Rows, at))
                }),
                // The hinge tilts it as far as the hand has moved down, its top towards the participant.
                Handle::Tilt => {
                    let tilt = tilted(start, down);
                    Some(start.clone().with_tilt((tilt != 0.0).then_some(tilt)))
                }
                // The text handle draws its content as much larger as the hand has moved to the right, and smaller to
                // the left, its slots as they were.
                Handle::Text => Some(content_at(start, content_scaled(start, right))),
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
                    // As its slots give it, and at either end, the text handle stops.
                    Handle::Text => {
                        let scale = changed.content_scale();
                        (scale - 1.0).abs() < 1e-4
                            || scale <= changed.least_content_scale() + 1e-4
                            || scale >= layout::CONTENT_SCALES.1 - 1e-4
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
                    content_scale: None,
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
                    content_scale: ((after.content_scale() - before.content_scale()).abs() > 1e-4)
                        .then(|| after.content_scale()),
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
                    || change.content_scale.is_some()
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
            let (points, _) = handle_points_among_with(wall, *frame, &all, keep, controls);
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
        // Or on the padlock of a locked wall it meets, which no handle does while it meets one.
        if let (Some((_, along)), Some(ray)) = (padlock_met, ray) {
            output.end = Some(ray.at(along));
            output.hit = Some(crate::pointing::End::new(along, ray.at(along)));
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
        output.sheets = self.sheets(walls, &padlocks, &input);
        if let Some((point, closer)) = locked_met {
            // A little before the wall, towards the viewer, so that the wall hides none of it.
            let before = point + (input.viewer - point).normalize_or_zero() * LIFT;
            output
                .sheets
                .push(hint_beside(locked_label(closer), before, input.viewer, 1.0));
        }
        output
    }

    /// Each wall's bar, for a participant who may adjust walls, highlighted while the ray meets it or its wall is
    /// carried, with what the grip does there while it is not taken hold of; each of `padlocks`, closed, highlighted with
    /// what the grip does while the ray meets it; and the handles of the wall taken hold of, each in the wall's plane
    /// where its frame puts it, and the hint above it, facing the viewer, as they are drawn.
    fn sheets(
        &mut self,
        walls: &[(&Wall, Mat4)],
        padlocks: &[(&Wall, Mat4, f32)],
        input: &Input<'_>,
    ) -> Vec<Sheet> {
        let mut sheets = Vec::new();
        for (wall, model, size) in padlocks {
            let on = self.padlock.as_deref() == Some(wall.id.as_str());
            let key = (Handle::Lock, on, true);
            let canvas = self
                .faces
                .entry(key)
                .or_insert_with(|| {
                    crate::frame_timing::drawn(handle_face(key.0, 1.0, key.1, key.2))
                })
                .clone();
            sheets.push(Sheet {
                model: Some(*model * Mat4::from_scale(Vec3::splat(*size))),
                canvas: Some(canvas),
            });
            if on {
                // Above the padlock, clear of it, as the padlock is above its wall.
                let up = model.transform_vector3(Vec3::Y).normalize_or(Vec3::Y);
                let edge = model.w_axis.truncate() + up * size * HANDLE_REACH;
                sheets.push(hint_beside(
                    bar_label_for(input.controls, wall, input.rights),
                    edge,
                    input.viewer,
                    1.0,
                ));
            }
        }
        let all: Vec<&Wall> = walls.iter().map(|(wall, _)| *wall).collect();
        self.bars
            .retain(|id, _| all.iter().any(|wall| wall.id == *id));
        if input.rights.adjust {
            for (wall, frame) in walls {
                let carried = self.carrying() == Some(wall.id.as_str());
                let (wall, frame) = match (&self.carry, &self.preview) {
                    (Some(carry), Some(preview)) if carried => (preview, carry.model),
                    _ => (*wall, *frame),
                };
                // A group shows one bar, below its bottom left wall.
                let Some(bar) = group_bar(wall, frame, &all) else {
                    continue;
                };
                let on = self.bar.as_deref() == Some(wall.id.as_str());
                let (key, scale) = bar_key(&bar, on || carried);
                let canvas = match self.bars.get(wall.id.as_str()) {
                    Some((kept, face)) if *kept == key => face.clone(),
                    _ => {
                        let face = crate::frame_timing::drawn(bar_face(key));
                        self.bars.insert(wall.id.clone(), (key, face.clone()));
                        face
                    }
                };
                sheets.push(Sheet {
                    model: Some(bar.model * Mat4::from_scale(Vec3::splat(scale))),
                    canvas: Some(canvas),
                });
                if on && self.editing() != Some(wall.id.as_str()) {
                    // Below the bar, clear of it, as the bar is below its wall.
                    let up = frame.transform_vector3(Vec3::Y).normalize_or(Vec3::Y);
                    let edge = bar.middle - up * bar.size * HANDLE_REACH;
                    sheets.push(hint_beside(
                        bar_label_for(input.controls, wall, input.rights),
                        edge,
                        input.viewer,
                        -1.0,
                    ));
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
                canvas: Some(crate::frame_timing::drawn(ghost(cells))),
                model: Some(slot * Mat4::from_translation(Vec3::Z * LIFT)),
            }));
        }
        let keep = self.drag.as_ref().map(|drag| drag.handle);
        let controls = LockControls::of(wall, input.rights);
        let (models, size) = handle_models(wall, frame, &all, keep, controls);
        let points: Vec<(Handle, Vec3)> = models
            .iter()
            .map(|(handle, model)| (*handle, model.w_axis.truncate()))
            .collect();
        if self.carry.is_none() {
            // Each face drawn once, a handle across, and kept, its model scaling it to the wall's handles.
            for (handle, model) in &models {
                let key = (*handle, self.hovered == Some(*handle), wall.is_locked());
                let canvas = self
                    .faces
                    .entry(key)
                    .or_insert_with(|| {
                        crate::frame_timing::drawn(handle_face(key.0, 1.0, key.1, key.2))
                    })
                    .clone();
                sheets.push(Sheet {
                    model: Some(*model * Mat4::from_scale(Vec3::splat(size))),
                    canvas: Some(canvas),
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
        let knob = points
            .iter()
            .find(|(handle, _)| *handle == Handle::Bend)
            .map_or(frame.transform_point3(Vec3::ZERO), |(_, at)| *at);
        // In a group, above its other walls' slots and its bar too, so that it covers nothing the group shows.
        let slots = super::boards::members_slots(wall, &all)
            .into_iter()
            .map(|slots| frame.transform_point3(Vec3::new(0.0, slots.top, 0.0)));
        // And above its bar, where its bar is raised above it, clear of the floor.
        let bars = walls
            .iter()
            .filter(|(other, _)| {
                other.id != wall.id && other.group.is_some() && other.group == wall.group
            })
            .filter_map(|(other, at)| group_bar(other, *at, &all))
            .chain(group_bar(wall, frame, &all))
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

/// What a bar's face is drawn from: its length in font pixels, and whether it is highlighted.
type BarFace = (usize, bool);

/// Eight font pixels thick, and as many long as it is.
const BAR_PIXELS: usize = 8;
const BAR_PIXEL: f32 = BAR_THICKNESS / BAR_PIXELS as f32;

/// What `bar`'s face is drawn from, `highlighted` or not, and the scale it is drawn at: drawn at one size, whatever the
/// bar's, so that it is the same face for walls of any size, and only its model scales, unless its length in font
/// pixels changes.
fn bar_key(bar: &Bar, highlighted: bool) -> (BarFace, f32) {
    let pixel = (BAR_THICKNESS * bar.size / BAR_PIXELS as f32).max(1e-5);
    let length = ((bar.half.length() / pixel).round() as usize).max(BAR_PIXELS);
    ((length, highlighted), pixel / BAR_PIXEL)
}

/// A bar's face, `length` font pixels either way and thick as a bar, subtle, and bright while `highlighted`.
fn bar_face((length, highlighted): BarFace) -> Canvas {
    let mut canvas = Canvas::new(0, BAR_PIXEL);
    canvas.padding = (length, BAR_PIXELS / 2);
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

/// What a locked wall the ray meets says, faintly, where the grip takes nothing: that it is locked, and, where coming
/// `closer` would show an owner its padlock, which they see only within [`PADLOCK_NEAR`] and in clear view, to come
/// closer to unlock it.
pub fn locked_label(closer: bool) -> Canvas {
    let text = if closer {
        "Locked: come closer to unlock"
    } else {
        "Locked"
    };
    let columns = crate::ui_text::width(text);
    let mut canvas = Canvas::new(columns, HINT_ANGLE);
    canvas.padding = (5, 4);
    canvas.background = [0.0, 0.0, 0.0, 0.45];
    canvas.rows = vec![Row::new(cut(
        &[crate::ui_text::Span::new(text, Style::plain(DIM))],
        columns,
    ))];
    canvas
}

/// What the grip does on a bar of `wall` for a participant with `rights`: takes hold of it to edit it, carrying one it
/// carries; or, on the padlock of a locked wall, which only an owner sees, unlocks it.
pub fn bar_label_for(controls: Controls, wall: &Wall, rights: Rights) -> Canvas {
    let grip = if controls == Controls::Keyboard {
        "G"
    } else {
        "Grip"
    };
    let action = if wall.is_locked() {
        "unlock this wall"
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
/// as `tilted 12° down`, or `upright`; for the text handle, the size of its content and how many columns and rows of a
/// panel's body each slot then shows, such as `Text 80% · 42 columns × 15 rows`; for an edge, its grid, such as
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
        Handle::Tilt => match wall.tilt() {
            tilt if tilt > 0.0 => format!("tilted {tilt:.0}° down"),
            tilt if tilt < 0.0 => format!("tilted {:.0}° up", -tilt),
            _ => "upright".to_owned(),
        },
        Handle::Text => {
            let cells = Cells::of(wall);
            format!(
                "Text {:.0}% · {} columns × {} rows",
                wall.content_scale() * 100.0,
                cells.columns,
                cells.body
            )
        }
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
fn hint_above(canvas: Canvas, above: Vec3, viewer: Vec3) -> Sheet {
    hint_beside(canvas, above, viewer, 1.0)
}

/// The hint `canvas` standing just above `edge`, `side` 1, or just below it, -1, facing `viewer`, as [`hint_above`]
/// draws it.
fn hint_beside(mut canvas: Canvas, edge: Vec3, viewer: Vec3, side: f32) -> Sheet {
    canvas.pixel = HINT_ANGLE * edge.distance(viewer).max(0.1);
    let (_, tall) = canvas.size();
    Sheet {
        model: Some(facing(edge + Vec3::Y * side * tall / 2.0, viewer, 1.0)),
        canvas: Some(crate::frame_timing::drawn(canvas)),
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
/// someone added where they remove walls, and the bar carries one they carry. Where the server locks walls, it says too,
/// for an owner, that the grip on the padlock locks it, and, for an environment's own, that the grip on `↺` puts it back,
/// locked; a locked wall is never taken hold of.
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
        rows.push((format!("{grip} aA"), "↔ text size".to_owned(), false));
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
