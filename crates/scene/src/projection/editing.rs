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
    Attach, Grid, Pose, Projection, Wall, WallKind, WallShape, group, wall as layout,
};

use super::{
    Cells, Controls, FRAME, HIGHLIGHT, HINT_ANGLE, LIFT, PENDING, THROW, ghost, slot_unit,
    two_hands,
};
#[cfg(test)]
use crate::canvas::Canvas;
use crate::{
    board::{model, pose},
    controller_ui::Sheet,
    locomotion::Locomotion,
    manipulation::Manipulation,
    pointing::Ray,
    recording::facing,
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

/// How far apart, at most, the places a locked wall's padlock may stand along its top and bottom edges are, in metres:
/// its corners, and between them along a wall wider than this, so that wherever an owner stands along it one is within
/// a metre of the point of its edge nearest them.
pub const PADLOCK_SPACING: f32 = 2.0;

/// How much nearer the viewer another of a locked wall's padlock's places must be than where it stands for it to move
/// there, in metres, so that it stays put as they move about and does not flicker between two.
pub const PADLOCK_STAY: f32 = 0.3;

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
    /// wall, for everyone. A locked wall, never taken hold of, shows it alone and closed to an owner near it, at
    /// whichever of its [`padlock_points`] is nearest them, whose squeeze unlocks it.
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
/// walls of its group, so that it is laid out again only when one of them changes. It holds every place the padlock may
/// stand, as [`padlock_points`] lays them out, and which of them it stands at, as the viewer last moved it.
#[derive(Clone, Debug)]
struct PadlockLayout {
    wall: Wall,
    frame: Mat4,
    members: Vec<Wall>,
    places: Vec<PadlockPlace>,
    size: f32,
    at: usize,
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

/// Both hands holding the wall taken hold of: where both took hold, the wall as it was as the second closed and where
/// it stood then, and whether it is at the least or the most its slots may be, so that reaching either ticks.
#[derive(Clone, Debug, PartialEq)]
struct Stretch {
    hold: two_hands::Hold,
    start: Wall,
    frame: Mat4,
    stopped: bool,
}

/// Whether both hands size `wall`'s slots as they spread or close, as its corners do: any wall but the Tomori wall,
/// whose slots are its own size, and a wall of a group, whose walls stand from one another as they are sized.
fn sized_by_hands(wall: &Wall) -> bool {
    wall.kind != WallKind::Tomori && wall.group.is_none()
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
    /// Where along its edge the padlock of the wall taken hold of stands, where an owner unlocked it there, so that it
    /// stays where it was rather than jumping to beyond its top left corner.
    lock_spot: Option<LockSpot>,
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
    /// Both hands holding the wall taken hold of.
    stretch: Option<Stretch>,
    /// The hand the editor follows holds the wall taken hold of by its board, slot or bar, squeezed on it, until it
    /// lets go, carried or not, so that the other hand may take hold of it too.
    gripped: bool,
    /// The wall's version as the second hand took hold of it, once both hands have held it, which a change of its slots'
    /// size is sent with, so that the server refuses it where the wall changed since.
    stretched: Option<u64>,
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
    /// The other hand's ray, while both hands hold the wall: they move and turn it between them, where it is carried,
    /// and size its slots as they spread or close.
    pub second: Option<Ray>,
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
pub(super) fn corners(wall: &Wall) -> [Vec3; 4] {
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

    /// Whether a wall is carried or a handle dragged, or both hands hold a wall, so that the hand doing it keeps the
    /// editor until it lets go.
    pub fn busy(&self) -> bool {
        self.drag.is_some() || self.carry.is_some() || self.stretch.is_some()
    }

    /// Whether both hands hold the wall taken hold of.
    pub fn stretching(&self) -> bool {
        self.stretch.is_some()
    }

    /// The wall the hand the editor follows holds, carried or squeezed by its board, slot or bar, that the other hand
    /// may take hold of too, by identity: one carried, or one whose slots both hands would size.
    pub fn held_wall(&self) -> Option<&str> {
        self.wall
            .as_deref()
            .filter(|_| self.drag.is_none() && (self.carry.is_some() || self.gripped))
    }

    /// Where the wall carried stands as it is drawn, its frame.
    pub fn carried_frame(&self) -> Option<Mat4> {
        self.carry.as_ref().map(|carry| carry.model)
    }

    /// The wall both hands hold, as it is drawn, and its frame.
    pub fn stretched(&self) -> Option<(Wall, Mat4)> {
        self.stretch
            .as_ref()
            .and(self.preview.as_ref())
            .and_then(|wall| {
                self.carry
                    .as_ref()
                    .map(|carry| carry.model)
                    .or_else(|| model(&wall.frame()))
                    .map(|frame| (wall.clone(), frame))
            })
    }

    /// The other hand, its ray `second`, takes hold of `wall`, standing at `frame` as drawn, which the hand the editor
    /// follows holds, as [`Self::held_wall`] says, its ray `first`, the point `along` its ray, so that both hold it.
    pub fn join(&mut self, first: Ray, second: Ray, along: f32, wall: &Wall, frame: Mat4) {
        let start = self.preview.clone().unwrap_or_else(|| wall.clone());
        let frame = self.carry.as_ref().map_or(frame, |carry| carry.model);
        // Carried, its hand holds it as far along its ray as it carries it; held where it stands, it does not move.
        let reach = self
            .carry
            .as_ref()
            .map_or(0.0, |carry| carry.manipulation.distance());
        self.stretched = Some(start.version);
        self.preview = Some(start.clone());
        self.stretch = Some(Stretch {
            hold: two_hands::Hold::new([first, second], [reach, along]),
            start,
            frame,
            stopped: true,
        });
    }

    /// One of the two hands holding the wall lets go, `ray` the ray of the one that still holds it: that hand holds it
    /// alone from where it is now, carrying it along its ray from there, without a jump, where it is carried.
    pub fn let_one_go(&mut self, ray: Option<Ray>) {
        self.stretch = None;
        for (carry, ray) in self.carry.iter_mut().zip(ray) {
            let along = (carry.model.w_axis.truncate() - ray.origin).dot(ray.direction);
            let manipulation = Manipulation::new(along);
            let at = ray.at(manipulation.distance());
            let (_, facing, _) = carry.model.to_scale_rotation_translation();
            carry.facing = facing;
            carry.grabbed = carry.model.inverse().transform_point3(at);
            carry.start = manipulation.distance();
            carry.manipulation = manipulation;
        }
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
                let (models, size) =
                    handle_models_at(wall, *frame, &all, None, controls, self.lock_spot);
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
        self.stretch = None;
        self.gripped = false;
        self.stretched = None;
        self.preview = None;
        self.members.clear();
        self.moved.clear();
        self.detached = false;
        self.snap = None;
        self.confirm = None;
        self.hovered = None;
        self.bar = None;
        self.padlock = None;
        self.lock_spot = None;
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
    /// wall, as its face's model and size, and whether it stands below its wall: for an owner, where the server locks
    /// walls, each at whichever of its [`padlock_points`] is nearest `viewer`, moving to another only where that is
    /// [`PADLOCK_STAY`] nearer, and shown only within [`PADLOCK_NEAR`] of `viewer` there, with none of `surfaces`
    /// between `viewer` and the padlock or the middle of its wall, so that a wall beneath a deck shows nothing to an
    /// owner on it, however near. Anyone else, and an owner further away, sees none, as a locked wall shows nothing
    /// else. Each is laid out once and kept until its wall, where it stands or its group changes; as the viewer moves,
    /// only which of its places it stands at changes.
    fn padlocks<'a>(
        &mut self,
        walls: &[(&'a Wall, Mat4)],
        viewer: Vec3,
        surfaces: Option<&Locomotion>,
        rights: Rights,
    ) -> Vec<(&'a Wall, Mat4, f32, bool)> {
        if !(rights.adjust && rights.locks && rights.lock) {
            return Vec::new();
        }
        let all: Vec<&Wall> = walls.iter().map(|(wall, _)| *wall).collect();
        self.padlock_layouts
            .retain(|id, _| all.iter().any(|wall| wall.is_locked() && wall.id == *id));
        let mut padlocks = Vec::new();
        for (wall, frame) in walls.iter().filter(|(wall, _)| wall.is_locked()) {
            let members = super::boards::members(wall, &all);
            let kept = self.padlock_layouts.get(&wall.id).is_some_and(|kept| {
                kept.wall == **wall
                    && kept.frame == *frame
                    && kept.members.iter().eq(members.iter().copied())
            });
            if !kept {
                let (places, size) = padlock_places(wall, *frame, &all);
                let at = nearest_place(
                    places.iter().map(|place| place.model.w_axis.truncate()),
                    viewer,
                );
                self.padlock_layouts.insert(
                    wall.id.clone(),
                    PadlockLayout {
                        wall: (*wall).clone(),
                        frame: *frame,
                        members: members.into_iter().cloned().collect(),
                        places,
                        size,
                        at,
                    },
                );
            }
            let Some(layout) = self
                .padlock_layouts
                .get_mut(&wall.id)
                .filter(|layout| !layout.places.is_empty())
            else {
                continue;
            };
            // It moves to the place nearest the viewer only where that is nearer by `PADLOCK_STAY` than where it is, or
            // at once where it would not be seen where it is, beyond reach or hidden, so that it shows wherever the
            // place nearest them would.
            let point = |index: usize| layout.places[index].model.w_axis.truncate();
            let distance = |index: usize| point(index).distance(viewer);
            let nearest = nearest_place(
                layout
                    .places
                    .iter()
                    .map(|place| place.model.w_axis.truncate()),
                viewer,
            );
            if distance(layout.at) > PADLOCK_NEAR
                || hidden(surfaces, viewer, point(layout.at))
                || distance(layout.at) - distance(nearest) > PADLOCK_STAY
            {
                layout.at = nearest;
            }
            let place = layout.places[layout.at];
            let padlock = place.model.w_axis.truncate();
            let middle = frame.w_axis.truncate();
            if padlock.distance(viewer) <= PADLOCK_NEAR
                && !hidden(surfaces, viewer, padlock)
                && !hidden(surfaces, viewer, middle)
            {
                padlocks.push((*wall, place.model, layout.size, place.below));
            }
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
            let (models, size) =
                handle_models_at(wall, *frame, &all, None, controls, self.lock_spot);
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
                    .filter_map(|(wall, model, size, _)| {
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
                            .chain(self.padlock_layouts.get(&wall.id).and_then(|kept| {
                                kept.places
                                    .get(kept.at)
                                    .map(|place| place.model.w_axis.truncate())
                            }))
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
                self.lock_spot = self
                    .padlock_layouts
                    .get(&wall.id)
                    .and_then(|kept| kept.places.get(kept.at))
                    .and_then(|place| place.spot);
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
                        let carries = carried(wall, input.rights) && !anchored;
                        // Held by its board, a wall both hands would carry or size may be taken hold of by the other
                        // hand too, until this one lets go.
                        self.gripped = carries || sized_by_hands(wall);
                        if carries {
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
        // Held with both hands, its slots are sized as they spread or close, about the middle of the points they took,
        // in the steps and within the bounds its corners size them in, and, carried, it moves and turns between them.
        let mut held_at = None;
        if let (Some(stretch), Some(first), Some(second)) = (
            &mut self.stretch,
            input.ray.filter(|_| input.active),
            input.second,
        ) {
            let moved = stretch.hold.follow([first, second]);
            let start = &stretch.start;
            let shown = self.preview.as_ref().map_or(start.scale(), Wall::scale);
            let mut changed = start.clone();
            let mut stopped = false;
            if sized_by_hands(start) {
                let scale = two_hands::settled(shown, start.scale() * moved.spread, layout::SCALES);
                let sized =
                    |scale: f32| laid_out(start, start.grid(), scale, start.aspect, start.radius());
                // Grown past what a wall may be, it stays as large as it was.
                changed = Some(sized(scale))
                    .filter(allowed)
                    .unwrap_or_else(|| sized(shown));
                stopped = (changed.scale() - layout::SCALES.0).abs() < 1e-4
                    || (changed.scale() - layout::SCALES.1).abs() < 1e-4
                    || (changed.scale() - scale).abs() > 1e-4;
            }
            output.ticked |= stopped && !stretch.stopped;
            stretch.stopped = stopped;
            let ratio = changed.scale() / start.scale();
            if let Some(carry) = &mut self.carry {
                carry.model = moved.frame(stretch.frame, ratio);
                let tilt = Mat4::from_rotation_x(changed.tilt().to_radians());
                changed.pose = pose(carry.model * tilt.inverse());
            }
            held_at = Some(first.at(stretch.hold.along()[0]));
            self.preview = Some(changed);
        }
        // A wall carried follows the ray, facing the viewer, as far along it and as turned as the stick puts it.
        if let (Some(carry), Some(ray)) = (&mut self.carry, input.ray.filter(|_| input.active)) {
            let stretching = held_at.is_some();
            if !stretching {
                output.ticked |= carry.manipulation.update(input.stick, input.dt);
            }
            let at = held_at.unwrap_or_else(|| ray.at(carry.manipulation.distance()));
            // Its frame is tilted as it tilts, which carrying keeps, and its pose is upright.
            let tilt = Mat4::from_rotation_x(
                self.preview
                    .as_ref()
                    .map_or(0.0, |preview| preview.tilt().to_radians()),
            );
            // Squared up, it faces the participant, its tilt kept, from where it is carried.
            if input.straighten && !stretching {
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
            if !stretching {
                carry.model = Mat4::from_translation(at)
                    * Mat4::from_rotation_y(carry.manipulation.yaw())
                    * Mat4::from_quat(carry.facing)
                    * Mat4::from_translation(-carry.grabbed);
                self.preview
                    .iter_mut()
                    .for_each(|preview| preview.pose = pose(carry.model * tilt.inverse()));
            }
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
            // Or as both hands took hold of it, its slots sized by them.
            self.stretch = None;
            self.gripped = false;
            let stretched = self.stretched.take();
            let sized = stretched.is_some()
                && self
                    .preview
                    .as_ref()
                    .zip(before.as_ref())
                    .is_some_and(|(after, before)| (after.scale() - before.scale()).abs() > 1e-4);
            let held = held.or(stretched.filter(|_| sized));
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
                (self.preview.take(), before, carried || dragged || sized)
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
                    // Dragged by a handle, or sized with both hands, it is changed from how it was taken hold of;
                    // carried, it is the participant's own wherever it was.
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
            let (models, _) = handle_models_at(wall, *frame, &all, keep, controls, self.lock_spot);
            let at = models
                .iter()
                .find(|(shown, _)| *shown == handle)
                .map_or(frame.transform_point3(Vec3::ZERO), |(_, model)| {
                    model.w_axis.truncate()
                });
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
        padlocks: &[(&Wall, Mat4, f32, bool)],
        input: &Input<'_>,
    ) -> Vec<Sheet> {
        let mut sheets = Vec::new();
        for (wall, model, size, below) in padlocks {
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
                // Beyond the padlock, clear of it, as the padlock is beyond its wall: above it, or below one below,
                // unless that would reach within `FLOOR_CLEARANCE` of the floor, when it stands above it too.
                let up = model.transform_vector3(Vec3::Y).normalize_or(Vec3::Y);
                let middle = model.w_axis.truncate();
                let label = bar_label_for(input.controls, wall, input.rights);
                let under = middle - up * size * HANDLE_REACH;
                let low = under.y - hint_height(&label, under, input.viewer)
                    < tomori_protocol::cluster::FLOOR + FLOOR_CLEARANCE;
                let (edge, side) = if *below && !low {
                    (under, -1.0)
                } else {
                    (middle + up * size * HANDLE_REACH, 1.0)
                };
                sheets.push(hint_beside(label, edge, input.viewer, side));
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
        let (models, size) = handle_models_at(wall, frame, &all, keep, controls, self.lock_spot);
        let points: Vec<(Handle, Vec3)> = models
            .iter()
            .map(|(handle, model)| (*handle, model.w_axis.truncate()))
            .collect();
        if self.carry.is_none() && self.stretch.is_none() {
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
        let hint = match (&self.drag, &self.stretch) {
            (Some(drag), _) => readout(drag.handle, wall),
            // Held with both hands, the size of its slots, as its corner says it.
            (None, Some(stretch)) if sized_by_hands(&stretch.start) => {
                readout(Handle::Corner(1, 1), wall)
            }
            _ => hint_for(
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

mod faces;
mod handles;

pub use faces::{
    Grouping, LOCKED, bar_label, bar_label_for, handle_face, hint, hint_for, hint_with,
    locked_label, padlock, readout,
};
pub use handles::{
    Bar, FLOOR_CLEARANCE, bar, group_bar, handle_point, handle_point_among, handle_points,
    handle_points_among, handle_points_among_with, handle_points_with, handle_size, padlock_point,
    padlock_points,
};

use faces::{BarFace, bar_face, bar_key, hint_above, hint_beside, hint_height};
use handles::{
    BAR_THICKNESS, LockSpot, PadlockPlace, handle_models_at, nearest_handle, nearest_place,
    padlock_places,
};
#[cfg(test)]
use {
    faces::glyph,
    handles::{BAR_LEAST, BAR_MOST, along, handle_models, handles},
};

#[cfg(test)]
mod group_controls_tests;
#[cfg(test)]
mod snapping_tests;
#[cfg(test)]
mod test_support;
#[cfg(test)]
mod tests;
