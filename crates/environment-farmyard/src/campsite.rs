// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The campsite beside the log seat at the lookout over the waterfall's pool: a bonfire of stacked logs in a ring of
//! stones on a clearing of packed earth, a second log seat across the fire from the lookout's, and four marshmallow
//! sticks leaning over that log towards the fire, which participants take with the grip, carry, toast over the fire and
//! put down again. See the crate's `context/campsite.md`.
//!
//! Its sticks are loose bodies of `tomori-physics`, as the beach's stones are, the physics proposal's second stage
//! (`docs/proposals/physics-and-stacking.md`): each the convex hull of its rod and its marshmallow ([`hull`],
//! [`bodies`]), taken and let go with either hand as any body is, whoever holds one controlling it, every client
//! drawing it from their hand; let go, the farmyard's server side simulates it, in a world of the campsite's own
//! ([`colliders`], [`waters`], [`settings`]), so that it falls, rolls and rests, or floats in the pool, and puts it
//! back on its log where it does not stay ([`goes_back`]). The side toasts the marshmallow of a stick held in the fire's
//! heat from where the holder's hand holds it ([`toasted`]), and shares how toasted each is ([`Shared`], [`Event`]).
use std::f32::consts::TAU;

use glam::{Mat4, Quat, Vec2, Vec3};
use serde::{Deserialize, Serialize};
use tomori_assets::{Limits, Material, Mesh, Model, Node, Transform};
use tomori_physics::message::{State, Update};
use tomori_physics::{Body, Placement, Settings, Static, Water};

use crate::backdrop::{self, facing};
use crate::layout;
use crate::props::{palette, scatter};
use crate::shapes::Shapes;
use crate::trail;

/// The middle of the fire pit, on the ground: beside the lookout's log seat, west of it, between the trail and the tree
/// ferns on the pool's bank.
pub const FIRE: Vec3 = Vec3::new(-4.45, 0.0, -14.85);
/// How far from [`FIRE`] the middle of each of the pit's stones stands, and how many there are.
pub const PIT: f32 = 0.42;
pub const PIT_STONES: u32 = 11;
/// How high the flames reach above the pit, which the fire's solid stands to.
pub const FLAMES: f32 = 0.75;
/// The middle of the campsite's clearing of packed earth is [`FIRE`], and this its radius.
pub const CLEARING: f32 = 1.65;
/// How far where participants stand at the campsite stays from the backdrop's middle, at most, so that from there the
/// whole backdrop is within the headset's far plane, a metre short of it: half a metre beyond the trail's
/// [`trail::REACH`] and its two metres, as nowhere nearer has room for a fire with seats about it.
pub const REACH: f32 = 12.6;

/// The log seat across the fire from the lookout's, on the fire's west side: its middle, the way it lies, how long it
/// is and its radius. The sticks lean over it.
pub fn seat() -> (Vec3, Vec2, f32, f32) {
    (
        Vec3::new(FIRE.x - 1.45, 0.0, FIRE.z + 0.35),
        Vec2::new(0.06, 1.0).normalize(),
        1.0,
        0.19,
    )
}

/// The height of the top of a log seat of `radius`, which lies on chocks.
pub fn seat_top(radius: f32) -> f32 {
    radius * 2.0 + 0.04
}

/// The point among the flames the fire is heard from, its light shines from and a marshmallow toasts best at.
pub fn heart() -> Vec3 {
    FIRE + Vec3::Y * 0.45
}

/// How many marshmallow sticks there are.
pub const STICKS: usize = 4;
/// How long a stick is, its radius, how far from its butt a hand holds it, and its marshmallow's length and radius,
/// whose middle is this far from its tip.
pub const STICK_LENGTH: f32 = 1.0;
pub const STICK_RADIUS: f32 = 0.011;
pub const HOLD: f32 = 0.15;
pub const MARSHMALLOW: (f32, f32) = (0.045, 0.019);
pub const MARSHMALLOW_INSET: f32 = 0.04;

/// Where a stick is: its butt, and the way along it from its butt to its tip, a unit vector.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub butt: Vec3,
    pub along: Vec3,
}

impl Pose {
    /// A stick held in a hand at `hand`, turned by `turn`, as a controller's grip pose is: along the hand's −Z, as a
    /// fist holds a stick pointing forward, [`HOLD`] from its butt, as [`grip`] holds its body.
    pub fn held(hand: Vec3, turn: Quat) -> Self {
        Self::of(&Placement::new(hand, turn.normalize()).then(&grip()))
    }

    /// The stick whose body is placed at `placement`: its body's middle is the stick's, and its −Z runs from its butt
    /// to its tip.
    pub fn of(placement: &Placement) -> Self {
        Self {
            butt: placement.transform_point(Vec3::Z * STICK_LENGTH * 0.5),
            along: (placement.rotation * Vec3::NEG_Z).normalize_or(Vec3::NEG_Z),
        }
    }

    /// Stick `index` leaning on its log, its butt on the ground west of the seat, resting over the seat's top, its
    /// marshmallow over the seat towards the fire.
    pub fn racked(index: usize) -> Self {
        let (middle, along, _, radius) = seat();
        let along = Vec3::new(along.x, 0.0, along.y);
        // Across the log, level, towards the fire.
        let to_fire = FIRE - middle;
        let towards = (to_fire - along * to_fire.dot(along)).normalize();
        let spread = (index as f32 - (STICKS as f32 - 1.0) * 0.5) * 0.2;
        // Its butt back from the seat's middle, on the ground, its shaft leaning at the angle that lays it on the log,
        // touching it, rather than through it: the log's axis as far from the shaft as their radii together.
        let back = 0.6;
        let butt = middle + along * spread - towards * back;
        let axis = radius + 0.04;
        let reach = (back * back + axis * axis).sqrt();
        let touching = (radius + STICK_RADIUS + 0.004) / reach;
        let lean = axis.atan2(back) + touching.asin();
        // A little splayed, as sticks leant together are, each turned a few degrees.
        let turn = Quat::from_rotation_y((scatter(index as u32 + 9700) - 0.5) * 0.14);
        Self {
            butt,
            along: turn * (towards * lean.cos() + Vec3::Y * lean.sin()),
        }
    }

    /// Its tip, and the middle of its marshmallow, and its own middle.
    pub fn tip(&self) -> Vec3 {
        self.butt + self.along * STICK_LENGTH
    }

    pub fn marshmallow(&self) -> Vec3 {
        self.butt + self.along * (STICK_LENGTH - MARSHMALLOW_INSET)
    }

    pub fn middle(&self) -> Vec3 {
        self.butt + self.along * (STICK_LENGTH * 0.5)
    }

    /// Where the stick's model, which runs from its butt at the origin along −Z, is placed, unturned about its length.
    pub fn transform(&self) -> Mat4 {
        Mat4::from_rotation_translation(Quat::from_rotation_arc(Vec3::NEG_Z, self.along), self.butt)
    }

    /// How far along `ray`, from `origin` the way `direction` goes, a unit vector, it passes within `reach` of the stick,
    /// if it does: so that the grip takes a stick the ray passes near, as a thin thing is hard to point at exactly.
    pub fn met(&self, origin: Vec3, direction: Vec3, reach: f32) -> Option<f32> {
        let (along, distance) = closest(origin, direction, self.butt, self.along, STICK_LENGTH);
        (distance <= reach && along >= 0.0).then_some(along)
    }

    /// How far `point` is from any part of the stick, from its butt to its tip.
    pub fn from_stick(&self, point: Vec3) -> f32 {
        let along = (point - self.butt).dot(self.along).clamp(0.0, STICK_LENGTH);
        point.distance(self.butt + self.along * along)
    }

    /// How far `point` is from the part of the stick a hand holds, from its butt to its middle.
    pub fn from_handle(&self, point: Vec3) -> f32 {
        let along = (point - self.butt)
            .dot(self.along)
            .clamp(0.0, STICK_LENGTH * 0.5);
        point.distance(self.butt + self.along * along)
    }
}

/// Where along a ray from `origin` going `direction` it passes nearest the segment from `start`, going `way` for
/// `length`, and how near.
fn closest(origin: Vec3, direction: Vec3, start: Vec3, way: Vec3, length: f32) -> (f32, f32) {
    let offset = origin - start;
    let (b, d, e) = (direction.dot(way), direction.dot(offset), way.dot(offset));
    let denominator = 1.0 - b * b;
    let on_segment = if denominator < 1e-6 {
        0.0
    } else {
        ((e - b * d) / denominator).clamp(0.0, length)
    };
    let along = (start + way * on_segment - origin).dot(direction).max(0.0);
    let on_ray = origin + direction * along;
    let on_segment = (on_ray - start).dot(way).clamp(0.0, length);
    (along, on_ray.distance(start + way * on_segment))
}

/// How hot it is at `point` for a marshmallow, from 0 to 1: hottest a hand's breadth above the embers, among the
/// flames' tips, and nothing beyond about 0.6 m of [`heart`].
pub fn heat(point: Vec3) -> f32 {
    let distance = point.distance(heart());
    if !distance.is_finite() {
        return 0.0;
    }
    (1.0 - (distance - 0.15) / 0.45).clamp(0.0, 1.0)
}

/// How much a marshmallow toasts each second at full [`heat`]: white to black in 20 seconds over the heart of the fire.
pub const TOAST_RATE: f32 = 0.05;
/// How toasted a marshmallow is when it turns golden, brown and black, from 0, white, to 1; and black, it catches,
/// burning with a little flame while it is still in the fire's heat.
pub const GOLDEN: f32 = 0.3;
pub const BROWN: f32 = 0.55;
pub const BLACK: f32 = 0.8;

/// How many shades a marshmallow is drawn in, from white to black.
pub const SHADES: usize = 8;

/// The shade a marshmallow toasted `toast` is drawn in, from 0, white, to [`SHADES`] less one, black.
pub fn shade(toast: f32) -> usize {
    let toast = if toast.is_finite() { toast } else { 0.0 };
    ((toast / BLACK * (SHADES - 1) as f32).round() as usize).min(SHADES - 1)
}

/// The colour of shade `shade`, linear RGB: white, through golden and brown, to black.
pub fn colour(shade: usize) -> Vec3 {
    const STOPS: [Vec3; 4] = [
        Vec3::new(0.86, 0.84, 0.78),
        Vec3::new(0.78, 0.5, 0.16),
        Vec3::new(0.32, 0.15, 0.05),
        Vec3::new(0.035, 0.03, 0.028),
    ];
    let toast = shade.min(SHADES - 1) as f32 / (SHADES - 1) as f32 * BLACK;
    let (from, to, t) = if toast <= GOLDEN {
        (0, 1, toast / GOLDEN)
    } else if toast <= BROWN {
        (1, 2, (toast - GOLDEN) / (BROWN - GOLDEN))
    } else {
        (2, 3, (toast - BROWN) / (BLACK - BROWN))
    };
    STOPS[from].lerp(STOPS[to], t)
}

/// What a marshmallow toasted `toast` looks like, as the agent is told it.
pub fn looks(toast: f32) -> &'static str {
    match toast {
        toast if toast >= BLACK => "burnt black",
        toast if toast >= BROWN => "toasted brown",
        toast if toast >= GOLDEN => "toasted golden",
        toast if toast > 0.08 => "just warming",
        _ => "fresh and white",
    }
}

/// Whether a marshmallow toasted `toast` at `at` is alight: black, and still in the fire's heat.
pub fn burning(toast: f32, at: Vec3) -> bool {
    toast >= BLACK && heat(at) > 0.0
}

/// How far toasting `toast` goes at `at` in `seconds`, as the side computes it from where the holder's hand holds the
/// stick, and the holder's client foresees it from where it draws it: by the fire's [`heat`] there, at most to 1.
pub fn toasted(toast: f32, at: Vec3, seconds: f32) -> f32 {
    let seconds = if seconds.is_finite() {
        seconds.max(0.0)
    } else {
        0.0
    };
    (toast + TOAST_RATE * heat(at) * seconds).clamp(0.0, 1.0)
}

/// The sticks are bodies of `tomori-physics`, of a world of the campsite's own, numbered from this, so that they are
/// told apart from the beach's stones in what participants ask.
pub const FIRST_STICK: u32 = 100;

/// The number of stick `index`'s body.
pub fn body(index: usize) -> u32 {
    FIRST_STICK + index as u32
}

/// Which stick body `body` is, if it is one.
pub fn index(body: u32) -> Option<usize> {
    body.checked_sub(FIRST_STICK)
        .map(|index| index as usize)
        .filter(|index| *index < STICKS)
}

/// How dense a stick is, with its marshmallow, in kilograms a cubic metre: dry wood, lighter than water, so that one
/// let go in the pool floats; and its friction and how much it bounces.
pub const DENSITY: f32 = 600.0;
pub const FRICTION: f32 = 0.7;
pub const RESTITUTION: f32 = 0.1;

/// The corners of a ring of `sides` about a length running `along`, at `at`, of `radius`, as the campsite's logs and
/// the stick's model draw their rings.
fn ring(along: Vec3, at: Vec3, radius: f32, sides: u32) -> Vec<Vec3> {
    let side = along.any_orthonormal_vector();
    let up = along.cross(side);
    (0..sides)
        .map(|index| {
            let angle = index as f32 / sides as f32 * TAU;
            at + (side * angle.cos() + up * angle.sin()) * radius
        })
        .collect()
}

/// How many sides a stick's rod and its marshmallow have, alike and turned alike, so that the faces of its hull between
/// them are of four corners, on which it lies still, rather than slivers it rocks on.
pub const STICK_SIDES: u32 = 6;

/// The points of a stick's hull, about its middle, its butt towards +Z and its tip towards −Z: the corners of its
/// rod's ends and of its marshmallow's, as its model draws them, so that it collides as it is drawn, its rod lying no
/// more than the marshmallow's few millimetres more than its own above what it rests on.
pub fn hull() -> Vec<Vec3> {
    let half = STICK_LENGTH * 0.5;
    let (length, radius) = MARSHMALLOW;
    let middle = Vec3::NEG_Z * (half - MARSHMALLOW_INSET);
    [
        (Vec3::Z * half, STICK_RADIUS),
        (Vec3::NEG_Z * half, STICK_RADIUS),
        (middle + Vec3::Z * length * 0.5, radius),
        (middle - Vec3::Z * length * 0.5, radius),
    ]
    .into_iter()
    .flat_map(|(at, radius)| ring(Vec3::NEG_Z, at, radius, STICK_SIDES))
    .collect()
}

impl Pose {
    /// Where the body of the stick this is is placed, as [`Pose::of`] reads it, turned about its length as `roll`.
    pub fn placement(&self, roll: f32) -> Placement {
        Placement::new(
            self.middle(),
            Quat::from_rotation_arc(Vec3::NEG_Z, self.along) * Quat::from_rotation_z(roll),
        )
    }
}

/// Where stick `index` begins, leaning on its log, as [`Pose::racked`] lays it: which the side settles, and to which a
/// stick put back goes.
pub fn home(index: usize) -> Placement {
    Pose::racked(index).placement(0.3)
}

/// Where a hand holds a stick's body, by its handle, [`HOLD`] from its butt, along the hand's −Z, as a fist holds a
/// stick pointing forward, whether it reached for it or pulled it along the ray.
pub fn grip() -> Placement {
    Placement::at(Vec3::new(0.0, 0.0, HOLD - STICK_LENGTH * 0.5))
}

/// Where the stick's model, which runs from its butt at the origin along −Z, is drawn for its body at `placement`.
pub fn drawn(placement: &Placement) -> Mat4 {
    placement.matrix() * Mat4::from_translation(Vec3::Z * STICK_LENGTH * 0.5)
}

/// The sticks as bodies, each of dry wood, leaning on its log.
pub fn bodies() -> Vec<Body> {
    (0..STICKS)
        .map(|index| {
            Body::convex(body(index), hull())
                .with_density(DENSITY)
                .with_friction(FRICTION)
                .with_restitution(RESTITUTION)
                .at(home(index))
        })
        .collect()
}

/// How far from [`FIRE`] what the sticks rest on reaches, taking in the pool below the campsite; beyond, a stick thrown
/// is lost, and goes back on its log at once.
pub const REGION: f32 = 13.0;

/// The corners of the ring of a log seat at `middle`, along `along`, of `radius`, at its end `end`, -1 or 1, as it is
/// drawn, on chocks.
fn log_ends((middle, along, length, radius): (Vec3, Vec2, f32, f32)) -> Vec<Vec3> {
    let along = Vec3::new(along.x, 0.0, along.y);
    let centre = middle + Vec3::Y * (radius + 0.04);
    [-0.5f32, 0.5]
        .into_iter()
        .flat_map(|end| ring(along, centre + along * length * end, radius, 8))
        .collect()
}

/// What the sticks rest on: the ground about the fire, the trail's track and the campsite's clearing, as they are drawn,
/// and the pool's bed, one grid of heights; both log seats; the ring of stones about the fire; and the bonfire.
pub fn colliders() -> Vec<Static> {
    let fire = Vec2::new(FIRE.x, FIRE.z);
    let spacing = 0.5;
    let columns = (REGION * 2.0 / spacing) as usize + 1;
    let corner = fire - Vec2::splat(REGION);
    let heights: Vec<f32> = (0..columns * columns)
        .map(|index| {
            let point =
                corner + Vec2::new((index % columns) as f32, (index / columns) as f32) * spacing;
            let ground = backdrop::drawn_ground(point).unwrap_or(backdrop::MEADOW);
            if trail::from_middle(point) < trail::HALF {
                ground.max(trail::LIFT)
            } else if point.distance(fire) < CLEARING {
                ground.max(trail::CLEARING_HEIGHT)
            } else {
                ground
            }
        })
        .collect();
    // One surface, the clearing's height in it, rather than the clearing's own triangles over it, which a stick lying
    // across both would creep on as they disagree by a millimetre.
    let mut colliders = vec![Static::heights(corner, spacing, columns, &heights)];
    for log in [trail::seat(), seat()] {
        colliders.push(Static::convex(log_ends(log)));
    }
    colliders.extend((0..PIT_STONES).map(|stone| {
        let (base, top) = pit_stone(stone);
        Static::convex(base.into_iter().chain(top).collect())
    }));
    colliders.push(Static::convex(bonfire().into_iter().flatten().collect()));
    colliders
}

/// How long a stick rests in the pool before it goes back on its log, in milliseconds, as a stone does in the river.
pub const RETURNS: u64 = crate::beach::RETURNS;

/// The pool below the campsite, in which a stick let go floats, and from which, once it has rested there a while, it
/// goes back on its log.
pub fn waters() -> Vec<Water> {
    let outline = (0..16)
        .map(|step| {
            let turn = step as f32 / 16.0 * TAU;
            Vec2::new(layout::POOL.x, layout::POOL.z)
                + Vec2::new(turn.cos(), turn.sin()) * layout::POOL_RADIUS
        })
        .collect();
    vec![Water::below(layout::POOL.y, outline).returning(RETURNS)]
}

/// How the campsite's world simulates: as `tomori-physics` does by default, a stick lost beyond its region going back
/// on its log.
pub fn settings() -> Settings {
    Settings {
        home: Vec3::new(FIRE.x, 0.0, FIRE.z),
        lost_beyond: REGION,
        ..Settings::default()
    }
}

/// How near where it began, leaning on its log, a stick at rest is back on its log, fresh.
pub const RACKED: f32 = 0.03;

/// How fast a stick may be let go, at most, in metres a second, to be put back on its log when let go by it, rather
/// than thrown from beside it.
pub const GENTLY: f32 = 1.0;

/// Whether a stick with its middle at `middle` is by its log, where one let go gently is put back on it.
pub fn by_its_log(middle: Vec3) -> bool {
    let (seat, ..) = seat();
    Vec2::new(middle.x, middle.z).distance(Vec2::new(seat.x, seat.z)) < BACK
}

/// Whether a stick with its middle at `middle` floats in the pool, from which it goes back on its log once it has rested
/// there for [`RETURNS`].
pub fn in_pool(middle: Vec3) -> bool {
    waters().iter().any(|water| water.contains(middle))
}

/// Whether a stick resting at `at` goes back on its log with a fresh marshmallow: by its log, its middle there; in the
/// fire, which takes it, any of it within the ring of stones; or carried off, its middle beyond [`DROP_REACH`] of the
/// fire; but for one floating in the pool, which goes back after a while.
pub fn goes_back(at: &Placement) -> bool {
    let middle = at.position;
    if in_pool(middle) {
        return false;
    }
    let pose = Pose::of(at);
    let flat = |point: Vec3| Vec2::new(point.x, point.z);
    let fire = flat(FIRE);
    let (butt, tip) = (flat(pose.butt), flat(pose.tip()));
    let way = tip - butt;
    let along = ((fire - butt).dot(way) / way.length_squared().max(1e-6)).clamp(0.0, 1.0);
    let nearest = (butt + way * along).distance(fire);
    let distance = flat(middle).distance(fire);
    !(distance.is_finite() && nearest.is_finite())
        || nearest < PIT
        || distance < INTO_FIRE
        || distance > DROP_REACH
        || by_its_log(middle)
}

/// How long the space is empty before the campsite tidies itself, every stick back on its log with a fresh marshmallow:
/// an hour.
pub const TIDY: std::time::Duration = std::time::Duration::from_secs(60 * 60);

/// How far from [`FIRE`], across the ground, a stick may come to rest: beyond it, as when carried off along the trail,
/// it goes back on its log.
pub const DROP_REACH: f32 = 5.0;

/// How near its log's middle, across the ground, the middle of a stick let go goes back on its log, and how near the
/// fire's middle it falls into the fire, which takes it, a fresh one appearing on the log.
pub const BACK: f32 = 0.75;
pub const INTO_FIRE: f32 = PIT + 0.15;

/// How toasted a stick's marshmallow is, as the side says it.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Toasted {
    pub body: u32,
    pub toast: f32,
}

/// What the farmyard shares of the campsite, in its state beside the flock's, as `campsite`: every stick's body, and
/// how toasted each is, in order.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Shared {
    #[serde(default)]
    pub sticks: State,
    #[serde(default)]
    pub toast: Vec<f32>,
}

/// The campsite as the farmyard shares it, in its state, or none in a state from before its sticks were bodies.
pub fn shared(state: &serde_json::Value) -> Shared {
    serde_json::from_value(state["campsite"].clone()).unwrap_or_default()
}

/// What changed of the campsite, as the side sends it to everyone in the space: the farmyard's clock, what changed of
/// the sticks' bodies, and how toasted those whose toasting changed are now. A client from before reads it as neither
/// a chicken's nor a stone's, and ignores it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub clock: u64,
    pub sticks: Update,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub toast: Vec<Toasted>,
}

/// The solids of the campsite, which the agent, walls and panels keep clear of, as `(least, greatest)` corners: the
/// fire pit with its flames, and the log seat across it with the sticks leaning on it.
pub fn solids() -> Vec<([f32; 3], [f32; 3])> {
    let reach = PIT + 0.12;
    let mut solids = vec![(
        [FIRE.x - reach, 0.0, FIRE.z - reach],
        [FIRE.x + reach, FLAMES, FIRE.z + reach],
    )];
    let (middle, along, length, radius) = seat();
    let along = Vec3::new(along.x, 0.0, along.y);
    let ends = [middle + along * length * 0.5, middle - along * length * 0.5];
    let mut low = ends[0].min(ends[1]) - Vec3::new(radius, 0.0, radius);
    let mut high = ends[0].max(ends[1]) + Vec3::new(radius, seat_top(radius), radius);
    for index in 0..STICKS {
        let pose = Pose::racked(index);
        for point in [pose.butt, pose.tip()] {
            low = low.min(point - Vec3::splat(MARSHMALLOW.1));
            high = high.max(point + Vec3::splat(MARSHMALLOW.1));
        }
    }
    solids.push(([low.x, 0.0, low.z], high.to_array()));
    solids
}

/// Where participants may stand about the fire, each the middle of a disc and its radius, at the trail's height: its
/// clearing's ring, outside the fire's stones and the seats, joined to the trail, on the fire's sides towards the trail
/// and the lookout's seat, so that they stand within [`REACH`]; beyond the fire, towards the pool, they sit.
pub fn standing() -> Vec<(Vec2, f32)> {
    let fire = Vec2::new(FIRE.x, FIRE.z);
    [5.6f32, 0.25, 1.2, 2.0]
        .into_iter()
        .map(|turn| (fire + Vec2::new(turn.sin(), turn.cos()) * 1.1, 0.55))
        .collect()
}

/// The room the campsite's anchor gives what gathers there, as width, height and depth: one to eighteen workers, clear
/// of the flames below and the ferns behind.
pub const ROOM: [f32; 3] = [4.4, 2.6, 1.0];

/// The campsite's named anchor, kind `area`, high above the fire, a little towards the trail from its middle, facing the
/// trail, so that workers gathered there stand over the flames, clear of them and of the tree ferns beyond, as seen from
/// whoever comes along it.
pub fn place() -> trail::Place {
    let fire = Vec2::new(FIRE.x, FIRE.z);
    let (point, _, _) = (0..trail::centreline().len())
        .map(trail::frame)
        .min_by(|a, b| a.0.distance(fire).total_cmp(&b.0.distance(fire)))
        .expect("the trail has points");
    let away = (fire - point).normalize();
    // Half a metre towards the lookout's seat, so that the room keeps clear of the tree ferns to the west.
    let at = fire - away * 0.3 + Vec2::X * 0.5;
    trail::Place {
        name: "campsite",
        kind: "area",
        label: "The campsite, with a bonfire and marshmallow sticks to toast",
        at: Vec3::new(at.x, 2.3, at.y),
        towards: Vec3::new(point.x, 2.3, point.y),
        notify_within: None,
    }
}

/// The colours of the campsite: its clearing's earth, its stones, the ash in its pit, its logs' bark and cut ends, and
/// what glows of the fire.
pub mod colors {
    use glam::Vec3;

    pub const CLEARING: Vec3 = Vec3::new(0.19, 0.135, 0.09);
    pub const STONE: Vec3 = Vec3::new(0.24, 0.23, 0.21);
    pub const ASH: Vec3 = Vec3::new(0.09, 0.085, 0.08);
    pub const CHARRED: Vec3 = Vec3::new(0.06, 0.045, 0.035);
    pub const EMBER: Vec3 = Vec3::new(1.0, 0.36, 0.08);
}

/// The campsite's materials: its plain one, lit, and the embers', glowing.
const EMBERS: usize = 1;

fn materials() -> Vec<Material> {
    vec![
        Material::new("plain", glam::Vec4::ONE),
        Material::new("embers", glam::Vec4::new(0.5, 0.2, 0.06, 1.0))
            .with_emissive(colors::EMBER * 1.2),
    ]
}

/// A log of `sides` sides from `from` to `to`, of `radius`, in `color` with its ends cut in `ends`.
fn log(
    shapes: &mut Shapes,
    (from, to): (Vec3, Vec3),
    radius: f32,
    sides: u32,
    (color, ends): (Vec3, Vec3),
) {
    let along = (to - from).normalize_or(Vec3::X);
    let side = along.any_orthonormal_vector();
    let up = along.cross(side);
    let ring = |at: Vec3| -> Vec<Vec3> {
        (0..sides)
            .map(|index| {
                let angle = index as f32 / sides as f32 * TAU;
                at + (side * angle.cos() + up * angle.sin()) * radius
            })
            .collect()
    };
    let (first, last) = (ring(from), ring(to));
    let middle = (from + to) * 0.5;
    for index in 0..sides as usize {
        let next = (index + 1) % sides as usize;
        let corners = [first[index], first[next], last[next], last[index]];
        let outward = corners.iter().copied().sum::<Vec3>() / 4.0 - middle;
        facing(shapes, &corners, outward, color);
    }
    facing(shapes, &first, -along, ends);
    facing(shapes, &last, along, ends);
}

/// The edge of the campsite's clearing, a corner every 30°, each where it is drawn: just beneath the trail's track,
/// where it meets it, or else just above the ground as the backdrop draws it, where it rises towards the pool.
pub fn clearing() -> Vec<Vec3> {
    (0..12)
        .map(|step| {
            let turn = step as f32 / 12.0 * TAU;
            let at = Vec2::new(FIRE.x, FIRE.z) + Vec2::new(turn.sin(), turn.cos()) * CLEARING;
            let ground = backdrop::drawn_ground(at)
                .unwrap_or(0.0)
                .max(backdrop::ground(at));
            Vec3::new(at.x, trail::CLEARING_HEIGHT.max(ground + 0.005), at.y)
        })
        .collect()
}

/// Where the middle of stone `stone` of the fire's ring stands, on the ground.
fn pit_middle(stone: u32) -> Vec3 {
    let turn = stone as f32 / PIT_STONES as f32 * TAU + scatter(stone + 9800) * 0.2;
    FIRE + Vec3::new(turn.sin(), 0.0, turn.cos()) * PIT
}

/// The corners of stone `stone` of the fire's ring, a low-poly boulder of five sides: those of its base, on the
/// clearing, and of its top, which it is drawn and collided from.
fn pit_stone(stone: u32) -> (Vec<Vec3>, Vec<Vec3>) {
    let lift = trail::CLEARING_HEIGHT;
    let turn = stone as f32 / PIT_STONES as f32 * TAU + scatter(stone + 9800) * 0.2;
    let at = pit_middle(stone);
    let size = 0.13 + scatter(stone + 9810) * 0.06;
    let sides = 5;
    let base: Vec<Vec3> = (0..sides)
        .map(|side| {
            let yaw = turn + side as f32 / sides as f32 * TAU;
            let reach = size * 0.5 * (0.8 + scatter(stone * 7 + side + 9820) * 0.4);
            at + Vec3::new(yaw.sin() * reach, lift, yaw.cos() * reach)
        })
        .collect();
    let height = size * (0.55 + scatter(stone + 9830) * 0.25);
    let top = base
        .iter()
        .map(|corner| {
            let inset = at + (*corner - at) * 0.55;
            Vec3::new(inset.x, lift + height, inset.z)
        })
        .collect();
    (base, top)
}

/// The bonfire's logs, five leaning together over the embers, their feet on the ash, and two laid across beneath them,
/// each from one end to the other.
fn bonfire() -> Vec<[Vec3; 2]> {
    let mut logs: Vec<[Vec3; 2]> = (0..5u32)
        .map(|index| {
            let turn = index as f32 / 5.0 * TAU + 0.4;
            let way = Vec3::new(turn.sin(), 0.0, turn.cos());
            let foot = FIRE + way * 0.27 + Vec3::Y * 0.03;
            let top = FIRE + way * 0.03 + Vec3::Y * (0.5 + scatter(index + 9850) * 0.08);
            [foot, top]
        })
        .collect();
    logs.extend([0.9f32, 2.6].into_iter().map(|turn| {
        let way = Vec3::new(turn.sin(), 0.0, turn.cos());
        let middle = FIRE + Vec3::Y * 0.06;
        [middle - way * 0.26, middle + way * 0.26]
    }));
    logs
}

/// The campsite's clearing, the fire's ring of stones, the ash in its pit, the second log seat and its chocks, and the
/// logs of the bonfire, the charred and the plain, in `shapes`, and its embers, which glow, in `embers`.
fn build(shapes: &mut Shapes, embers: &mut Shapes) {
    let lift = trail::CLEARING_HEIGHT;
    // The clearing, beneath the trail's track where it meets it, and above the ground as drawn where it rises.
    let edge = clearing();
    let middle = Vec3::new(FIRE.x, lift, FIRE.z);
    for index in 0..edge.len() {
        let corners = [middle, edge[index], edge[(index + 1) % edge.len()]];
        facing(shapes, &corners, Vec3::Y, colors::CLEARING);
    }
    shapes.disc(
        Vec3::new(FIRE.x, lift + 0.004, FIRE.z),
        PIT - 0.04,
        9,
        colors::ASH,
    );
    // The ring of stones, each a low-poly boulder, set about the pit.
    for stone in 0..PIT_STONES {
        let (base, top) = pit_stone(stone);
        let sides = base.len();
        let height = top[0].y - lift;
        let at = pit_middle(stone);
        let color = colors::STONE * (0.85 + scatter(stone + 9840) * 0.3);
        let middle = at + Vec3::Y * (height * 0.5);
        for side in 0..sides {
            let next = (side + 1) % sides;
            let corners = [base[side], base[next], top[next], top[side]];
            let outward = corners.iter().copied().sum::<Vec3>() / 4.0 - middle;
            facing(shapes, &corners, outward, color * 0.85);
        }
        facing(shapes, &top, Vec3::Y, color);
    }
    // The bonfire: five logs leaning together over the embers, their feet on the ash, charred towards the middle, and
    // two laid across beneath them, burning through.
    for (index, ends) in bonfire().into_iter().enumerate() {
        let (radius, colors) = if index < 5 {
            (0.04, (palette::BARK * 0.9, colors::CHARRED))
        } else {
            let shade = 1.0 + (index - 5) as f32 * 0.4;
            (0.05, (colors::CHARRED * shade, colors::CHARRED))
        };
        log(shapes, (ends[0], ends[1]), radius, 5, colors);
    }
    // The embers: a glowing bed in the pit, and coals among the logs' feet.
    embers.disc(
        Vec3::new(FIRE.x, lift + 0.012, FIRE.z),
        0.2,
        7,
        colors::EMBER,
    );
    for coal in 0..6u32 {
        let turn = coal as f32 / 6.0 * TAU + scatter(coal + 9860);
        let at =
            FIRE + Vec3::new(turn.sin(), 0.0, turn.cos()) * (0.12 + scatter(coal + 9870) * 0.12);
        embers.cone(at + Vec3::Y * lift, 0.035, 0.035, 4, colors::EMBER);
    }
    // The second log seat, on two chocks, as the lookout's is.
    let (middle, along, length, radius) = seat();
    let along = Vec3::new(along.x, 0.0, along.y);
    let centre = middle + Vec3::Y * (radius + 0.04);
    log(
        shapes,
        (centre - along * length * 0.5, centre + along * length * 0.5),
        radius,
        8,
        (palette::BARK * 1.2, palette::WOOD),
    );
    for end in [-0.32, 0.32] {
        let chock = middle + along * length * end;
        shapes.cuboid(
            chock + Vec3::Y * 0.04,
            Vec3::new(0.06, 0.04, radius * 0.9),
            Quat::from_rotation_arc(Vec3::X, along),
            palette::DARK_WOOD,
        );
    }
}

/// The campsite, as a model of two nodes: its clearing, stones, ash and logs, lit, and its embers, which glow.
pub fn model() -> Model {
    let (mut shapes, mut embers) = (Shapes::new(), Shapes::new());
    build(&mut shapes, &mut embers);
    Model::built(
        vec![
            Node::new("campsite", Transform::IDENTITY).with_mesh(0),
            Node::new("embers", Transform::IDENTITY).with_mesh(1),
        ],
        vec![0, 1],
        vec![
            Mesh::new("campsite", vec![shapes.primitive(0)]),
            Mesh::new("embers", vec![embers.primitive(EMBERS)]),
        ],
        materials(),
        &Limits::default(),
    )
    .expect("the campsite is built within the limits")
}

/// The name of the node of the stick's model that draws its marshmallow in shade `shade`.
pub fn shade_node(shade: usize) -> String {
    format!("marshmallow-{shade}")
}

/// The model of a marshmallow stick, its butt at the origin, running along −Z: its stick, and its marshmallow in each
/// of [`SHADES`], of which an instance shows one. The little flame of one that burns is the fire's, which flickers.
pub fn stick() -> Model {
    let mut wood = Shapes::new();
    log(
        &mut wood,
        (Vec3::ZERO, Vec3::NEG_Z * STICK_LENGTH),
        STICK_RADIUS,
        STICK_SIDES,
        (palette::WOOD * 1.1, palette::WEATHERED),
    );
    let (length, radius) = MARSHMALLOW;
    let middle = Vec3::NEG_Z * (STICK_LENGTH - MARSHMALLOW_INSET);
    let span = (
        middle + Vec3::Z * length * 0.5,
        middle - Vec3::Z * length * 0.5,
    );
    let mut nodes = vec![Node::new("stick", Transform::IDENTITY).with_mesh(0)];
    let mut meshes = vec![Mesh::new("stick", vec![wood.primitive(0)])];
    for shade in 0..SHADES {
        let mut marshmallow = Shapes::new();
        let colour = colour(shade);
        log(
            &mut marshmallow,
            span,
            radius,
            STICK_SIDES,
            (colour, colour * 1.08),
        );
        nodes.push(Node::new(shade_node(shade), Transform::IDENTITY).with_mesh(meshes.len()));
        meshes.push(Mesh::new(shade_node(shade), vec![marshmallow.primitive(0)]));
    }
    let roots = (0..nodes.len()).collect();
    Model::built(nodes, roots, meshes, materials(), &Limits::default())
        .expect("the stick is built within the limits")
}

/// The most triangles the campsite, its sticks and its fire's flames and embers draw together, which the tests check.
pub const BUDGET: usize = 800;

#[cfg(test)]
mod tests;
