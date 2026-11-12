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
use std::f32::consts::{PI, TAU};

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
/// How far the fire reaches from its heart, as a diagram's line to it rings it: its pit of stones, and a little more.
pub const FIRE_EXTENT: f32 = PIT + 0.13;

/// How far what the agent is told of as `reference` reaches from where it is said to be, as a diagram's line to it
/// rings it, with the fire at `size`: the bonfire [`FIRE_EXTENT`], as much further as its flames have grown
/// ([`reach`]); the woodpile to its stone's corners; the fireside log half its length; and a stick [`STICK_EXTENT`].
pub fn extent(reference: &str, size: f32) -> f32 {
    match reference {
        "campsite/bonfire" => FIRE_EXTENT * reach(size),
        "campsite/woodpile" => PILE * std::f32::consts::SQRT_2,
        "campsite/fireside-log" => fireside().2 * 0.5,
        _ => STICK_EXTENT,
    }
}
/// The middle of the campsite's clearing of packed earth is [`FIRE`], and this its radius.
pub const CLEARING: f32 = 1.65;
/// How far where participants stand at the campsite stays from the backdrop's middle, at most, so that from there the
/// whole backdrop is within its 50 m bound from the eyes, a metre short of it: three quarters of a metre beyond the trail's
/// [`trail::REACH`] and its two metres, as nowhere nearer has room for a fire with seats about it that participants
/// walk all the way round.
pub const REACH: f32 = 12.75;

/// How far from [`FIRE`] its clearing is walked, at most: short of its edge, so that the ground ferns about it stay
/// beyond the clearance from where anyone stands.
pub const WALKED: f32 = CLEARING - 0.15;

/// The log seat across the fire from the lookout's, on the fire's west side: its middle, the way it lies, how long it
/// is and its radius. The sticks lean over it. Far enough from the fire that participants walk between the two.
pub fn seat() -> (Vec3, Vec2, f32, f32) {
    (
        Vec3::new(FIRE.x - 1.65, 0.0, FIRE.z + 0.35),
        Vec2::new(0.06, 1.0).normalize(),
        1.0,
        0.19,
    )
}

/// How far from [`FIRE`] the fireside log lies, towards the pool.
pub const FIRESIDE: f32 = 1.65;

/// The fireside log, a third log seat, between the fire and the pool, across the fire from the trail, facing the fire:
/// its middle, on the ground as drawn there, the way it lies, how long it is and its radius, as [`seat`] gives the
/// second's. Participants come to sit on it by pointing at it ([`targets`]).
pub fn fireside() -> (Vec3, Vec2, f32, f32) {
    let fire = Vec2::new(FIRE.x, FIRE.z);
    let towards = (Vec2::new(layout::POOL.x, layout::POOL.z) - fire).normalize();
    let at = fire + towards * FIRESIDE;
    let along = towards.perp();
    let length = 1.2;
    // On the higher of the ground beneath its chocks, which rises gently towards the pool, so that neither floats.
    let ground = [-0.32f32, 0.32]
        .into_iter()
        .map(|end| {
            backdrop::drawn_ground(at + along * length * end)
                .unwrap_or(backdrop::MEADOW)
                .max(trail::CLEARING_HEIGHT)
        })
        .fold(f32::NEG_INFINITY, f32::max);
    (Vec3::new(at.x, ground, at.y), along, length, 0.18)
}

/// The height of the top of a log seat of `radius`, which lies on chocks, above the ground it lies on.
pub fn seat_top(radius: f32) -> f32 {
    radius * 2.0 + 0.04
}

/// The point among the flames the fire is heard from, its light shines from and a marshmallow toasts best at, at its
/// default size.
pub fn heart() -> Vec3 {
    heart_at(0.0)
}

/// The fire's [`heart`] at `size` ([`flames`]): as much higher as its toasting reaches further.
pub fn heart_at(size: f32) -> Vec3 {
    FIRE + Vec3::Y * 0.45 * reach(size)
}

/// How many marshmallow sticks there are.
pub const STICKS: usize = 4;
/// How long a stick is, its radius, how far from its butt a hand holds it, and its marshmallow's length and radius,
/// whose middle is this far from its tip: a big one, 9 cm long, its far end 17.5 mm short of the tip, which shows
/// through it.
pub const STICK_LENGTH: f32 = 1.0;
pub const STICK_RADIUS: f32 = 0.011;
pub const HOLD: f32 = 0.15;
pub const MARSHMALLOW: (f32, f32) = (0.09, 0.038);
pub const MARSHMALLOW_INSET: f32 = 0.0625;
/// How far a stick reaches from where it is described, its middle, as a diagram's line to it rings it.
pub const STICK_EXTENT: f32 = STICK_LENGTH / 2.0;

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
        // touching it, rather than through it: the log's axis as far from the shaft as the flats of the log's eight
        // sides and of the stick's six where it crosses it, at its hull's girth there. Far enough back that it crosses
        // the log well beyond its middle of mass, which its big marshmallow draws towards its tip, so that it rests on
        // the log rather than tipping over it.
        let back = 0.72;
        let butt = middle + along * spread - towards * back;
        let ground = backdrop::drawn_ground(Vec2::new(butt.x, butt.z)).unwrap_or(backdrop::MEADOW);
        let butt = Vec3::new(butt.x, ground + STICK_RADIUS, butt.z);
        let axis = middle.y + radius + 0.04 - butt.y;
        let reach = (back * back + axis * axis).sqrt();
        let flats = radius * (PI / 8.0).cos() + girth(reach) * (PI / STICK_SIDES as f32).cos();
        let touching = flats / reach;
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

/// How hot it is at `point` for a marshmallow at the fire's default size, from 0 to 1: hottest a hand's breadth above
/// the embers, among the flames' tips, and nothing beyond about 0.6 m of [`heart`].
pub fn heat(point: Vec3) -> f32 {
    heat_at(point, 0.0)
}

/// How hot it is at `point` for a marshmallow with the fire at `size`: as at its default size, but as much further
/// from its [`heart_at`] as its toasting reaches ([`reach`]), so that a bigger fire toasts from further off.
pub fn heat_at(point: Vec3, size: f32) -> f32 {
    let reach = reach(size);
    let distance = point.distance(heart_at(size));
    if !distance.is_finite() {
        return 0.0;
    }
    (1.0 - (distance - 0.15 * reach) / (0.45 * reach)).clamp(0.0, 1.0)
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

/// Whether a marshmallow toasted `toast` at `at` catches, or burns on as long as it can, with the fire at its default
/// size: black, in the fire's heat. Out of it, it burns on as [`burns`] says.
pub fn burning(toast: f32, at: Vec3) -> bool {
    burning_at(toast, at, 0.0)
}

/// Whether a marshmallow toasted `toast` at `at` catches, or burns on as long as it can, with the fire at `size`: black,
/// in the fire's heat ([`heat_at`]).
pub fn burning_at(toast: f32, at: Vec3, size: f32) -> bool {
    toast >= BLACK && heat_at(at, size) > 0.0
}

/// How long a marshmallow that has caught burns on out of the fire's heat, in seconds, before it goes out: long enough
/// to write a few words in the air with it, as with an ember on a stick, which the client crate draws as its trail.
pub const BURNS: f32 = 45.0;

/// How long a marshmallow toasted `toast`, at `at`, burns on, `seconds` after it would have burned on for `left`, with
/// the fire at its default size, as [`burns_at`] says.
pub fn burns(left: f32, toast: f32, at: Vec3, seconds: f32) -> f32 {
    burns_at(left, toast, at, seconds, 0.0)
}

/// How long a marshmallow toasted `toast`, at `at`, burns on, `seconds` after it would have burned on for `left`, with
/// the fire at `size`, as each client draws it: for [`BURNS`] while it is [`burning_at`] in the fire's heat, and less
/// as it is out of it, until it goes out on its own, or at once under the pool's water; and never unless it is black.
pub fn burns_at(left: f32, toast: f32, at: Vec3, seconds: f32, size: f32) -> f32 {
    if burning_at(toast, at, size) {
        return BURNS;
    }
    if toast.is_nan() || toast < BLACK {
        return 0.0;
    }
    let left = if left.is_finite() {
        (left - duration(seconds)).max(0.0)
    } else {
        0.0
    };
    if doused(at) { 0.0 } else { left.min(BURNS) }
}

/// Whether `point` is under the pool's water, which puts out a burning marshmallow.
pub fn doused(point: Vec3) -> bool {
    point.y < layout::POOL.y
        && Vec2::new(point.x - layout::POOL.x, point.z - layout::POOL.z).length()
            < layout::POOL_RADIUS
}

/// How far toasting `toast` goes at `at` in `seconds` with the fire at its default size.
pub fn toasted(toast: f32, at: Vec3, seconds: f32) -> f32 {
    toasted_at(toast, at, seconds, 0.0)
}

/// How far toasting `toast` goes at `at` in `seconds` with the fire at `size`, as the side computes it from where the
/// holder's hand holds the stick, and the holder's client foresees it from where it draws it: by the fire's
/// [`heat_at`] there, at most to 1.
pub fn toasted_at(toast: f32, at: Vec3, seconds: f32, size: f32) -> f32 {
    (toast + TOAST_RATE * heat_at(at, size) * duration(seconds)).clamp(0.0, 1.0)
}

/// `value` from 0 to 1, as a fire's size or how far a log has burnt is: nothing for nonsense.
pub fn unit(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// `seconds`, as long as a step of the fire, toasting or burning takes: none for less than none, or nonsense.
fn duration(seconds: f32) -> f32 {
    if seconds.is_finite() {
        seconds.max(0.0)
    } else {
        0.0
    }
}

/// How big the fire's flames are with the fire at `size`, from 0, its default, to 1, the biggest wood makes it, as a
/// scale of the flames as the bonfire was first drawn: a little smaller by default, and twice that at its biggest.
pub fn flames(size: f32) -> f32 {
    SMALL + (BIG - SMALL) * unit(size)
}

/// How big the flames are at the fire's default size and at its biggest ([`flames`]).
pub const SMALL: f32 = 0.8;
pub const BIG: f32 = 1.6;

/// How far a marshmallow toasts from the fire at `size`, as a scale of how far at its default size: as much further as
/// its flames are bigger, so that toasting by default is as it always was, and a fire at its biggest toasts from twice
/// as far.
pub fn reach(size: f32) -> f32 {
    flames(size) / SMALL
}

/// How many logs the woodpile beside the fire holds, which participants put on the fire to make it bigger.
pub const LOGS: usize = 8;
/// The logs are bodies of the campsite's world, with the sticks, numbered from this.
pub const FIRST_LOG: u32 = 110;

/// The number of log `index`'s body.
pub fn log_body(index: usize) -> u32 {
    FIRST_LOG + index as u32
}

/// Which log body `body` is, if it is one.
pub fn log_index(body: u32) -> Option<usize> {
    body.checked_sub(FIRST_LOG)
        .map(|index| index as usize)
        .filter(|index| *index < LOGS)
}

/// How long a log is, how far its six sides' corners are from its middle, and how many sides it has: a length of split
/// firewood, held in one hand.
pub const LOG_LENGTH: f32 = 0.42;
pub const LOG_RADIUS: f32 = 0.05;
pub const LOG_SIDES: u32 = 6;

/// How deep a log lies on one of its sides, from flat to flat.
pub fn log_depth() -> f32 {
    LOG_RADIUS * 3f32.sqrt()
}

/// The points of a log's hull, about its middle, along X: the corners of its ends, six-sided, a side flat beneath it
/// and one flat on top, so that logs crossed on each other rest flat on flat, faces of four corners.
pub fn log_hull() -> Vec<Vec3> {
    [-0.5f32, 0.5]
        .into_iter()
        .flat_map(|end| {
            (0..LOG_SIDES).map(move |side| {
                let angle = side as f32 / LOG_SIDES as f32 * TAU;
                Vec3::new(
                    end * LOG_LENGTH,
                    angle.sin() * LOG_RADIUS,
                    angle.cos() * LOG_RADIUS,
                )
            })
        })
        .collect()
}

/// The middle of the woodpile's flat stone, on the ground: between the second seat's end and the fireside log's, a
/// step beyond the ring participants walk round the fire, within reach of it and of both seats.
pub fn woodpile() -> Vec3 {
    let at = Vec2::new(FIRE.x, FIRE.z) + Vec2::new(-0.725, -0.69).normalize() * 1.75;
    let ground = [
        Vec2::new(-1.0, -1.0),
        Vec2::new(1.0, -1.0),
        Vec2::new(1.0, 1.0),
        Vec2::new(-1.0, 1.0),
    ]
    .into_iter()
    .map(|corner| {
        backdrop::drawn_ground(at + corner * PILE)
            .unwrap_or(backdrop::MEADOW)
            .max(trail::CLEARING_HEIGHT)
    })
    .fold(f32::NEG_INFINITY, f32::max);
    Vec3::new(at.x, ground, at.y)
}

/// How far the woodpile's stone reaches from its middle each way, and how far its flat top stands above the highest of
/// the ground beneath it.
pub const PILE: f32 = 0.27;
pub const PILE_TOP: f32 = 0.06;

/// Where log `index` begins, on the woodpile: stacked in pairs, each pair across the one beneath, as a crib of firewood
/// is, so that each log rests flat across the two beneath it near their ends; four pairs high.
pub fn log_home(index: usize) -> Placement {
    let pile = woodpile();
    let layer = (index / 2) as f32;
    let side = if index.is_multiple_of(2) { -1.0 } else { 1.0 };
    // A fifth of a millimetre apart, so that none begins within another.
    let height = pile.y + PILE_TOP + log_depth() * (layer + 0.5) + 0.0002 * (layer + 1.0);
    let spread = 0.13;
    if (index / 2).is_multiple_of(2) {
        // Along Z, apart along X.
        Placement::new(
            Vec3::new(pile.x + side * spread, height, pile.z),
            Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
        )
    } else {
        Placement::new(
            Vec3::new(pile.x, height, pile.z + side * spread),
            Quat::IDENTITY,
        )
    }
}

/// The woodpile's stone, as a collider and as it is drawn: its least and greatest corners, its bottom sunk into the
/// ground beneath it.
pub fn pile_stone() -> (Vec3, Vec3) {
    let pile = woodpile();
    (
        pile - Vec3::new(PILE, 0.08, PILE),
        pile + Vec3::new(PILE, PILE_TOP, PILE),
    )
}

/// How long a log burns, in seconds, from catching to burnt away, charring, glowing and shrinking as it goes; how long
/// it takes to catch, over which it feeds the fire; and how much of the fire's whole size each log feeds it.
pub const BURN: f32 = 40.0;
pub const CATCH: f32 = 6.0;
pub const FUEL: f32 = 0.4;
/// How long the fire takes to die back from its biggest to its default size, without more wood, in seconds.
pub const DIES_BACK: f32 = 240.0;

/// How far a log that has burnt `burnt` of the way has burnt `seconds` on, at most all the way.
pub fn burnt(burnt: f32, seconds: f32) -> f32 {
    (burnt + duration(seconds) / BURN).clamp(0.0, 1.0)
}

/// Whether a log that has burnt `burnt` of the way is still catching, feeding the fire.
pub fn catching(burnt: f32) -> bool {
    burnt > 0.0 && burnt * BURN < CATCH
}

/// The fire's size `seconds` on from `size`, with `catching` logs catching: each feeding it [`FUEL`] over [`CATCH`],
/// while it dies back over [`DIES_BACK`], from 0, its default, to 1, its biggest.
pub fn grown(size: f32, catching: usize, seconds: f32) -> f32 {
    unit(unit(size) + duration(seconds) * (catching as f32 * FUEL / CATCH - 1.0 / DIES_BACK))
}

/// Whether a log resting at `at` lies in the fire, which takes it: any of its length within the ring of stones, and
/// low among the flames rather than held above them.
pub fn in_fire(at: &Placement) -> bool {
    let flat = |point: Vec3| Vec2::new(point.x, point.z);
    let (from, to) = (
        flat(at.transform_point(Vec3::X * LOG_LENGTH * 0.5)),
        flat(at.transform_point(Vec3::NEG_X * LOG_LENGTH * 0.5)),
    );
    let fire = flat(FIRE);
    let way = to - from;
    let along = ((fire - from).dot(way) / way.length_squared().max(1e-6)).clamp(0.0, 1.0);
    let nearest = (from + way * along).distance(fire);
    nearest.is_finite() && nearest < PIT && at.position.y < FLAMES
}

/// Whether a log resting at `at`, not alight, goes back on the woodpile: carried off beyond [`DROP_REACH`] of the fire,
/// or nowhere at all. One lying about the fire stays, for anyone to put on it.
pub fn log_goes_back(at: &Placement) -> bool {
    let distance = Vec2::new(at.position.x, at.position.z).distance(Vec2::new(FIRE.x, FIRE.z));
    !(distance.is_finite() && at.position.y.is_finite()) || distance > DROP_REACH
}

/// What the fire looks like at `size`, as the agent is told it.
pub fn blaze(size: f32) -> &'static str {
    match size {
        size if size >= 0.75 => "blazing",
        size if size >= 0.4 => "burning high",
        size if size >= 0.1 => "burning well",
        _ => "burning low",
    }
}

/// How many shades a log is drawn in as it burns: its bark, charred, and glowing.
pub const LOG_SHADES: usize = 3;

/// The shade a log that has burnt `burnt` of the way is drawn in: its bark until it catches, charred as it catches,
/// and glowing once it has caught.
pub fn log_shade(burnt: f32) -> usize {
    match burnt {
        burnt if burnt * BURN >= CATCH => 2,
        burnt if burnt > 0.0 => 1,
        _ => 0,
    }
}

/// How large a log that has burnt `burnt` of the way is drawn, about its middle: shrinking to a third as it burns away.
pub fn charred(burnt: f32) -> f32 {
    1.0 - 0.7 * unit(burnt)
}

/// Where a log placed at `placement` runs: one end, and the way along it to the other, a unit vector.
fn log_axis(placement: &Placement) -> (Vec3, Vec3) {
    let way = (placement.rotation * Vec3::X).normalize_or(Vec3::X);
    (placement.position - way * LOG_LENGTH * 0.5, way)
}

/// How far along `ray`, from `origin` the way `direction` goes, a unit vector, it passes within `reach` of the log
/// placed at `placement`, beyond its sides, if it does.
pub fn log_met(placement: &Placement, origin: Vec3, direction: Vec3, reach: f32) -> Option<f32> {
    let (start, way) = log_axis(placement);
    let (along, distance) = closest(origin, direction, start, way, LOG_LENGTH);
    (distance <= LOG_RADIUS + reach && along >= 0.0).then_some(along)
}

/// How far `point` is from the sides of the log placed at `placement`, or nothing within them.
pub fn from_log(placement: &Placement, point: Vec3) -> f32 {
    let (start, way) = log_axis(placement);
    let along = (point - start).dot(way).clamp(0.0, LOG_LENGTH);
    (point.distance(start + way * along) - LOG_RADIUS).max(0.0)
}

/// The name of the node of the log's model that draws it in shade `shade`.
pub fn log_node(shade: usize) -> String {
    format!("log-{shade}")
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

/// How far a stick's hull reaches from its line, at most, `along` from its butt, up to its marshmallow: its rod's
/// radius at its butt, widening to its marshmallow's at the marshmallow's near end, as the hull is a cone between
/// them, on which it rests on what it leans on.
pub fn girth(along: f32) -> f32 {
    let (length, radius) = MARSHMALLOW;
    let near = STICK_LENGTH - MARSHMALLOW_INSET - length * 0.5;
    STICK_RADIUS + (radius - STICK_RADIUS) * (along / near).clamp(0.0, 1.0)
}

/// The points of a stick's hull, about its middle, its butt towards +Z and its tip towards −Z: the corners of its
/// rod's ends and of its marshmallow's, as its model draws them, so that it collides as it is drawn, its rod lying no
/// more than the marshmallow's radius more than its own above what it rests on, as a stick would with a marshmallow
/// on its end, and resting on its log by its hull's [`girth`] there.
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

/// The sticks as bodies, each of dry wood, leaning on its log; then the logs, of dry wood too, on the woodpile.
pub fn bodies() -> Vec<Body> {
    let sticks = (0..STICKS).map(|index| {
        Body::convex(body(index), hull())
            .with_density(DENSITY)
            .with_friction(FRICTION)
            .with_restitution(RESTITUTION)
            .at(home(index))
    });
    let logs = (0..LOGS).map(|index| {
        Body::convex(log_body(index), log_hull())
            .with_density(DENSITY)
            .with_friction(LOG_FRICTION)
            .with_restitution(RESTITUTION)
            .at(log_home(index))
    });
    sticks.chain(logs).collect()
}

/// How much a log grips what it rests on: rough split wood.
pub const LOG_FRICTION: f32 = 0.8;

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

/// What the sticks and logs rest on: the ground about the fire, the trail's track and the campsite's clearing, as they
/// are drawn, and the pool's bed, one grid of heights; the three log seats; the ring of stones about the fire; the
/// bonfire; and the woodpile's stone.
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
    for log in [trail::seat(), seat(), fireside()] {
        colliders.push(Static::convex(log_ends(log)));
    }
    colliders.extend((0..PIT_STONES).map(|stone| {
        let (base, top) = pit_stone(stone);
        Static::convex(base.into_iter().chain(top).collect())
    }));
    colliders.push(Static::convex(bonfire().into_iter().flatten().collect()));
    let (least, greatest) = pile_stone();
    colliders.push(Static::cuboid(least, greatest, Quat::IDENTITY));
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
/// on its log, but unclustered, as the beach's is, since one plane for the ground's triangles near alike pushed a stick
/// lying across them, its big marshmallow on its tip, round its butt a few millimetres a second, for ever.
pub fn settings() -> Settings {
    Settings {
        home: Vec3::new(FIRE.x, 0.0, FIRE.z),
        lost_beyond: REGION,
        ..Settings::default()
    }
    .unclustered()
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

/// Whether a stick placed at `at` floats in the pool, from which it goes back on its log once it has rested there for
/// [`RETURNS`]: its middle within the pool, and some of it below the water's surface, as `tomori-physics` soaks a body
/// lighter than the water, since one floating with its big marshmallow down may hold its middle above it.
pub fn in_pool(at: &Placement) -> bool {
    let lowest = hull()
        .into_iter()
        .map(|point| at.transform_point(point).y)
        .fold(f32::INFINITY, f32::min);
    let middle = Vec2::new(at.position.x, at.position.z);
    waters()
        .iter()
        .any(|water| lowest < water.height && tomori_physics::body::within(&water.outline, middle))
}

/// Whether a stick resting at `at` goes back on its log with a fresh marshmallow: by its log, its middle there; in the
/// fire, which takes it, any of it within the ring of stones; or carried off, its middle beyond [`DROP_REACH`] of the
/// fire; but for one floating in the pool, which goes back after a while.
pub fn goes_back(at: &Placement) -> bool {
    let middle = at.position;
    if in_pool(at) {
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

/// How far a log has burnt, as the side says it.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Burnt {
    pub body: u32,
    pub burnt: f32,
}

/// What the farmyard shares of the campsite, in its state beside the flock's, as `campsite`: the bodies of its world,
/// every stick's and then every log's, as `sticks`; how toasted each stick is, in order; how big the fire is, from 0,
/// its default, to 1; and how far each log has burnt, in order, from 0, not alight, to 1, burnt away.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Shared {
    #[serde(default)]
    pub sticks: State,
    #[serde(default)]
    pub toast: Vec<f32>,
    #[serde(default)]
    pub size: f32,
    #[serde(default)]
    pub burnt: Vec<f32>,
}

/// The campsite as the farmyard shares it, in its state, or none in a state from before its sticks were bodies.
pub fn shared(state: &serde_json::Value) -> Shared {
    serde_json::from_value(state["campsite"].clone()).unwrap_or_default()
}

/// What changed of the campsite, as the side sends it to everyone in the space: the farmyard's clock, what changed of
/// the sticks' and logs' bodies, how toasted those sticks whose toasting changed are now, how big the fire is, if that
/// changed, and how far the logs whose burning changed have burnt. A client from before reads it as neither a
/// chicken's nor a stone's, and ignores it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub clock: u64,
    pub sticks: Update,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub toast: Vec<Toasted>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<f32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub burnt: Vec<Burnt>,
}

/// The solids of the campsite, which the agent, walls and panels keep clear of, as `(least, greatest)` corners: the
/// fire pit with its flames, and what stands about it ([`furniture`]).
pub fn solids() -> Vec<([f32; 3], [f32; 3])> {
    let reach = PIT + 0.12;
    let mut solids = vec![(
        [FIRE.x - reach, 0.0, FIRE.z - reach],
        [FIRE.x + reach, FLAMES, FIRE.z + reach],
    )];
    solids.extend(furniture());
    solids
}

/// The ring of stones about the fire pit, seen from above: every corner of its stones' bases, X and Z, whose hull
/// participants keep clear of, rather than the box about the pit and its flames that the agent keeps clear of, so that
/// they walk round the fire between it and its seats.
pub fn pit() -> Vec<Vec2> {
    (0..PIT_STONES)
        .flat_map(|stone| pit_stone(stone).0)
        .map(|corner| Vec2::new(corner.x, corner.z))
        .collect()
}

/// The campsite's clearing, as it is drawn, which the farmyard's walkable ground takes in, so that participants walk
/// all the way round the fire: the part of each triangle of its fan within [`WALKED`] of the fire and [`REACH`] of the
/// backdrop's middle, a triangle in the same plane, reaching the trail and a metre from the fire towards the pool.
/// The fire's stones and what stands about it are cut from it as the walkable ground is derived.
pub fn ground() -> Vec<[Vec3; 3]> {
    let edge = clearing();
    let middle = Vec3::new(FIRE.x, trail::CLEARING_HEIGHT, FIRE.z);
    // How far towards `corner` from the middle stays within reach: where |d + w t| is the reach, at most all the way.
    let walked = |corner: Vec3| {
        let way = corner - middle;
        let (d, w) = (
            Vec2::new(middle.x, middle.z) - layout::BACKDROP_CENTRE,
            Vec2::new(way.x, way.z),
        );
        let (a, b, c) = (
            w.length_squared(),
            d.dot(w),
            d.length_squared() - REACH * REACH,
        );
        let t = (-b + (b * b - a * c).max(0.0).sqrt()) / a;
        middle + way * t.clamp(0.0, WALKED / CLEARING)
    };
    (0..edge.len())
        .map(|index| {
            let (from, to) = (edge[index], edge[(index + 1) % edge.len()]);
            [middle, walked(from), walked(to)]
        })
        .collect()
}

/// What stands about the fire, as `(least, greatest)` corners, which participants, the agent, walls and panels keep
/// clear of: the log seat across it with the sticks leaning on it, the woodpile and the fireside log.
pub fn furniture() -> Vec<([f32; 3], [f32; 3])> {
    let mut solids = Vec::new();
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
    // The woodpile, its stone and its logs stacked on it.
    let (least, greatest) = pile_stone();
    solids.push((
        [least.x, greatest.y - PILE_TOP, least.z],
        [
            greatest.x,
            greatest.y + log_depth() * (LOGS / 2) as f32,
            greatest.z,
        ],
    ));
    // The fireside log.
    let (middle, along, length, radius) = fireside();
    let along = Vec3::new(along.x, 0.0, along.y);
    let ends = [middle + along * length * 0.5, middle - along * length * 0.5];
    let low = ends[0].min(ends[1]) - Vec3::new(radius, 0.0, radius);
    let high = ends[0].max(ends[1]) + Vec3::new(radius, seat_top(radius), radius);
    solids.push(([low.x, middle.y - 0.05, low.z], high.to_array()));
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

/// How far about where participants arrive on the fireside log ([`seated`]) they stand, a small disc of ground beneath
/// its seat that the client declares, so that a target may take them there.
pub const SEAT: f32 = 0.2;

/// Where on the fireside log participants arrive, seated, on the ground beneath its seat, a little before its middle,
/// towards the fire, so that one sitting where they are sits on it, their eyes where one sitting on it has them, facing
/// the fire; and which way they face.
pub fn seated() -> (Vec3, Vec3) {
    let (middle, ..) = fireside();
    let towards = ((FIRE - middle) * Vec3::new(1.0, 0.0, 1.0)).normalize();
    let arrival = middle + towards * 0.12;
    (arrival, (FIRE - arrival) * Vec3::new(1.0, 0.0, 1.0))
}

/// The campsite's teleport target: the fireside log, pointed at, along most of its length and from its chocks up to a
/// little above its top, from anywhere within reach, taking one to sit on it, facing the fire ([`seated`]), on the
/// small disc of ground beneath its seat ([`SEAT`]), which no walkable ground reaches.
pub fn targets() -> [trail::TargetPlace; 1] {
    let (middle, _, length, radius) = fireside();
    let (arrival, facing) = seated();
    [trail::TargetPlace {
        foot: middle,
        radius: length * 0.5 - 0.1,
        height: seat_top(radius) + 0.1,
        arrival,
        name: "Fireside log",
        facing,
        within: None,
    }]
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
    // The woodpile's flat stone, its bottom sunk into the ground.
    let (least, greatest) = pile_stone();
    shapes.cuboid(
        (least + greatest) * 0.5,
        (greatest - least) * 0.5,
        Quat::IDENTITY,
        colors::STONE * 0.9,
    );
    // The second log seat and the fireside log, each on two chocks, as the lookout's is, sunk a little into the ground
    // where it rises beneath them.
    for (middle, along, length, radius) in [seat(), fireside()] {
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
                chock + Vec3::Y * 0.02,
                Vec3::new(0.06, 0.06, radius * 0.9),
                Quat::from_rotation_arc(Vec3::X, along),
                palette::DARK_WOOD,
            );
        }
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

/// The model of a log of the woodpile, about its middle, along X, in each of [`LOG_SHADES`], of which an instance shows
/// one: its bark, with its cut ends pale; charred; and glowing, as embers do. Drawn from its hull's corners, as it
/// collides.
pub fn log_model() -> Model {
    let corners = log_hull();
    let sides = LOG_SIDES as usize;
    let shades = [
        (palette::BARK * 1.1, palette::WOOD, 0),
        (colors::CHARRED * 1.4, colors::CHARRED, 0),
        (colors::EMBER * 0.6, colors::EMBER * 0.8, EMBERS),
    ];
    let mut nodes = Vec::new();
    let mut meshes = Vec::new();
    for (shade, (bark, ends, material)) in shades.into_iter().enumerate() {
        let mut shapes = Shapes::new();
        for side in 0..sides {
            let next = (side + 1) % sides;
            let quad = [
                corners[side],
                corners[next],
                corners[sides + next],
                corners[sides + side],
            ];
            let outward = quad.iter().copied().sum::<Vec3>() / 4.0 * Vec3::new(0.0, 1.0, 1.0);
            facing(&mut shapes, &quad, outward, bark);
        }
        facing(&mut shapes, &corners[..sides], Vec3::NEG_X, ends);
        facing(&mut shapes, &corners[sides..], Vec3::X, ends);
        nodes.push(Node::new(log_node(shade), Transform::IDENTITY).with_mesh(meshes.len()));
        meshes.push(Mesh::new(log_node(shade), vec![shapes.primitive(material)]));
    }
    let roots = (0..nodes.len()).collect();
    Model::built(nodes, roots, meshes, materials(), &Limits::default())
        .expect("the log is built within the limits")
}

/// The most triangles the campsite, its sticks and its fire's flames and embers draw together, which the tests check.
pub const BUDGET: usize = 1000;

#[cfg(test)]
mod tests;
