// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The campsite beside the log seat at the lookout over the waterfall's pool: a bonfire of stacked logs in a ring of
//! stones on a clearing of packed earth, a second log seat across the fire from the lookout's, and four marshmallow
//! sticks leaning over that log towards the fire, which participants take with the grip, carry, toast over the fire and
//! put down again. See the crate's `context/campsite.md`.
//!
//! Its sticks follow the authority of the physics proposal's first stage (`docs/proposals/physics-and-stacking.md`):
//! whoever holds a stick controls it, every client drawing it from their hand, which presence already brings, and
//! saying how toasted it is; let go, the farmyard's server side decides where it rests, straight down onto what is
//! below it, or back on its log with a fresh marshmallow, and owns it at rest. [`Campsite`] is that side's part, and
//! [`Stick`], [`Intent`] and [`Event`] what it shares, is asked and announces.
use std::f32::consts::TAU;

use glam::{Mat4, Quat, Vec2, Vec3};
use serde::{Deserialize, Serialize};
use tomori_assets::{Limits, Material, Mesh, Model, Node, Transform};

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
    /// fist holds a stick pointing forward, [`HOLD`] from its butt.
    pub fn held(hand: Vec3, turn: Quat) -> Self {
        let along = (turn * Vec3::NEG_Z).normalize_or(Vec3::NEG_Z);
        Self {
            butt: hand - along * HOLD,
            along,
        }
    }

    /// A stick lying as `lying` says.
    pub fn lying(lying: &Lying) -> Self {
        let along = Vec3::new(lying.yaw.sin(), 0.0, -lying.yaw.cos());
        Self {
            butt: Vec3::from(lying.at) - along * STICK_LENGTH * 0.5,
            along,
        }
    }

    /// Stick `index` leaning on its log, its butt on the ground west of the seat, resting over the seat's top, its
    /// marshmallow over the seat towards the fire.
    pub fn racked(index: usize) -> Self {
        let (middle, along, _, radius) = seat();
        let across = Vec3::new(along.y, 0.0, -along.x);
        let towards = if across.dot(FIRE - middle) > 0.0 {
            across
        } else {
            -across
        };
        let spread = (index as f32 - (STICKS as f32 - 1.0) * 0.5) * 0.27;
        let along = Vec3::new(along.x, 0.0, along.y);
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

    /// Its heading about the vertical, from −Z towards +X, as [`Lying::yaw`] says it.
    pub fn yaw(&self) -> f32 {
        self.along.x.atan2(-self.along.z)
    }

    /// Where the stick's model, which runs from its butt at the origin along −Z, is placed.
    pub fn transform(&self) -> Mat4 {
        Mat4::from_rotation_translation(Quat::from_rotation_arc(Vec3::NEG_Z, self.along), self.butt)
    }

    /// How far along `ray`, from `origin` the way `direction` goes, a unit vector, it passes within `reach` of the stick,
    /// if it does: so that the grip takes a stick the ray passes near, as a thin thing is hard to point at exactly.
    pub fn met(&self, origin: Vec3, direction: Vec3, reach: f32) -> Option<f32> {
        let (along, distance) = closest(origin, direction, self.butt, self.along, STICK_LENGTH);
        (distance <= reach && along >= 0.0).then_some(along)
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

/// A stick lying where it was let go: the middle of its length, and its heading about the vertical, from −Z towards +X.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Lying {
    pub at: [f32; 3],
    pub yaw: f32,
}

/// A marshmallow stick, as everyone in the space is told of it: its epoch, which each taking and letting go raises, and
/// which every intent about it names, so that two taking it at once are told apart; how toasted its marshmallow is;
/// who holds it, by their number, if anyone does; and where it lies, if it was let go anywhere but on its log.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Stick {
    pub epoch: u32,
    pub toast: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub held: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lying: Option<Lying>,
}

impl Stick {
    /// Where it is when nobody holds it: lying where it was let go, or on its log, as stick `index`.
    pub fn resting(&self, index: usize) -> Pose {
        match &self.lying {
            Some(lying) => Pose::lying(lying),
            None => Pose::racked(index),
        }
    }

    /// Whether it is on its log with a fresh marshmallow, nobody holding it.
    pub fn fresh(&self) -> bool {
        self.held.is_none() && self.lying.is_none() && self.toast == 0.0
    }
}

/// What the farmyard shares of the campsite, in its state beside the flock's: every stick, in order.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Shared {
    #[serde(default)]
    pub sticks: Vec<Stick>,
}

/// A stick that changed, as the side sends it to everyone in the space: the farmyard's clock, which stick, and what it
/// is now.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub clock: u64,
    pub stick: usize,
    pub now: Stick,
}

/// What a participant asks of the campsite: to take a stick, as their grip closes on it, at the epoch they saw; to say
/// how toasted the marshmallow they hold is; or to let it go, as drawn in their hand, with how toasted it is. Each
/// names the stick and its epoch, and only its holder's are taken but the first.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Intent {
    Grab {
        grab: usize,
        epoch: u32,
    },
    Toast {
        toast: usize,
        epoch: u32,
        level: f32,
    },
    Release {
        release: usize,
        epoch: u32,
        level: f32,
        at: [f32; 3],
        yaw: f32,
    },
}

/// How long the space is empty before the campsite tidies itself, every stick back on its log with a fresh marshmallow:
/// an hour, in milliseconds.
pub const TIDY: u64 = 60 * 60 * 1000;

/// How far from [`FIRE`], across the ground, a stick let go may lie: beyond it, as when carried off along the trail, it
/// goes back on its log.
pub const DROP_REACH: f32 = 5.0;

/// How near its log's middle, across the ground, the middle of a stick let go goes back on its log, and how near the
/// fire's middle it falls into the fire, which takes it, a fresh one appearing on the log.
pub const BACK: f32 = 0.75;
pub const INTO_FIRE: f32 = PIT + 0.15;

/// How much toasting the side takes as a holder says, beyond what [`TOAST_RATE`] could have done since they last said,
/// for messages that arrive unevenly.
const TOAST_SLACK: f32 = 0.03;

/// Whether `point` lies on the log seat from `middle` along `along`, `length` long, of `radius`, across the ground.
fn on_log(point: Vec2, (middle, along, length, radius): (Vec3, Vec2, f32, f32)) -> bool {
    let offset = point - Vec2::new(middle.x, middle.z);
    offset.dot(along).abs() <= length * 0.5 && offset.perp_dot(along).abs() <= radius
}

/// The height of whatever is beneath `point`, across the ground, that a stick let go there comes to rest on: the top of
/// either log seat, the trail's track or the campsite's clearing, or the meadow; or `None` where it does not rest, in
/// the fire, by its log, inside the fence or too far away, when it goes back on its log.
pub fn beneath(point: Vec2) -> Option<f32> {
    let fire = Vec2::new(FIRE.x, FIRE.z);
    let (middle, ..) = seat();
    if !point.is_finite()
        || point.distance(fire) > DROP_REACH
        || point.distance(fire) < INTO_FIRE
        || point.distance(Vec2::new(middle.x, middle.z)) < BACK
        || layout::FENCE.grown(0.3).contains(point)
    {
        return None;
    }
    let mut height = backdrop::ground(point);
    if trail::from_middle(point) < trail::HALF {
        height = height.max(trail::LIFT);
    } else if point.distance(fire) < CLEARING {
        height = height.max(trail::CLEARING_HEIGHT);
    }
    for log in [trail::seat(), seat()] {
        if on_log(point, log) {
            height = height.max(seat_top(log.3));
        }
    }
    Some(height)
}

/// Where a stick let go with its middle at `at`, heading `yaw`, comes to rest: straight down, lying level, on whatever
/// is beneath its middle, keeping its heading; or `None`, back on its log, where [`beneath`] says it does not rest.
pub fn rest(at: [f32; 3], yaw: f32) -> Option<Lying> {
    let point = Vec2::new(at[0], at[2]);
    let height = beneath(point)?;
    let yaw = if yaw.is_finite() { yaw } else { 0.0 };
    Some(Lying {
        at: [point.x, height + STICK_RADIUS, point.y],
        yaw,
    })
}

/// The campsite as the farmyard's server side keeps it: every stick, when each holder last said how toasted their
/// marshmallow is, and since when the space has been empty, if it is.
#[derive(Clone, Debug)]
pub struct Campsite {
    sticks: Vec<Stick>,
    toasted: Vec<u64>,
    empty: Option<u64>,
}

impl Default for Campsite {
    fn default() -> Self {
        Self::new()
    }
}

impl Campsite {
    /// The campsite with every stick on its log, fresh.
    pub fn new() -> Self {
        Self {
            sticks: vec![Stick::default(); STICKS],
            toasted: vec![0; STICKS],
            empty: None,
        }
    }

    pub fn sticks(&self) -> &[Stick] {
        &self.sticks
    }

    /// What it shares.
    pub fn shared(&self) -> Shared {
        Shared {
            sticks: self.sticks.clone(),
        }
    }

    /// The toasting the holder of stick `index` says, `level`, at `now`, as the side takes it: no less than it was, and
    /// no more than the fire could have toasted it since they last said.
    fn toasted(&mut self, index: usize, level: f32, now: u64) -> f32 {
        let was = self.sticks[index].toast;
        let since = now.saturating_sub(self.toasted[index]) as f32 / 1000.0;
        self.toasted[index] = now;
        if !level.is_finite() {
            return was;
        }
        level
            .min(was + TOAST_RATE * since + TOAST_SLACK)
            .clamp(was, 1.0)
    }

    /// Stick `index` back on its log, with a fresh marshmallow, nobody holding it.
    fn rack(&mut self, index: usize) {
        let stick = &mut self.sticks[index];
        stick.epoch = stick.epoch.wrapping_add(1);
        stick.held = None;
        stick.lying = None;
        stick.toast = 0.0;
    }

    /// Do what `participant` asks, at `now` on the farmyard's clock, if the side takes it, returning the stick it
    /// changed: a grab of a stick nobody holds at the epoch they saw, by someone holding no other; and the holder's
    /// toasting and letting go, at the epoch their grab made. Anything else, as a second grab of a stick just taken, is
    /// refused, changing nothing, so that the first grab to arrive wins.
    pub fn ask(&mut self, participant: u32, intent: Intent, now: u64) -> Option<usize> {
        let index = match intent {
            Intent::Grab { grab, .. } => grab,
            Intent::Toast { toast, .. } => toast,
            Intent::Release { release, .. } => release,
        };
        let holding = self
            .sticks
            .iter()
            .any(|stick| stick.held == Some(participant));
        let stick = self.sticks.get(index)?;
        match intent {
            Intent::Grab { epoch, .. } => {
                if stick.held.is_some() || stick.epoch != epoch || holding {
                    return None;
                }
                let stick = &mut self.sticks[index];
                stick.epoch = stick.epoch.wrapping_add(1);
                stick.held = Some(participant);
                self.toasted[index] = now;
            }
            Intent::Toast { epoch, level, .. } => {
                if stick.held != Some(participant) || stick.epoch != epoch {
                    return None;
                }
                let toast = self.toasted(index, level, now);
                if toast == self.sticks[index].toast {
                    return None;
                }
                self.sticks[index].toast = toast;
            }
            Intent::Release {
                epoch,
                level,
                at,
                yaw,
                ..
            } => {
                if stick.held != Some(participant) || stick.epoch != epoch {
                    return None;
                }
                let toast = self.toasted(index, level, now);
                match rest(at, yaw) {
                    Some(lying) => {
                        let stick = &mut self.sticks[index];
                        stick.epoch = stick.epoch.wrapping_add(1);
                        stick.held = None;
                        stick.lying = Some(lying);
                        stick.toast = toast;
                    }
                    None => self.rack(index),
                }
            }
        }
        Some(index)
    }

    /// Look at who is in the space, `present`, by their numbers, as the server last said, at `now`: a stick held by
    /// someone no longer there goes back on its log, fresh, and once nobody has been there for [`TIDY`], every stick
    /// does. Returns the sticks it changed. Nothing changes while the server has not said who is there.
    pub fn sweep(&mut self, present: Option<&[u32]>, now: u64) -> Vec<usize> {
        let Some(present) = present else {
            return Vec::new();
        };
        let mut changed: Vec<usize> = (0..self.sticks.len())
            .filter(|index| {
                self.sticks[*index]
                    .held
                    .is_some_and(|holder| !present.contains(&holder))
            })
            .collect();
        for index in &changed {
            self.rack(*index);
        }
        if present.is_empty() {
            let since = *self.empty.get_or_insert(now);
            if now.saturating_sub(since) >= TIDY {
                for index in 0..self.sticks.len() {
                    if !self.sticks[index].fresh() {
                        self.rack(index);
                        changed.push(index);
                    }
                }
            }
        } else {
            self.empty = None;
        }
        changed
    }

    /// What the space's agent is told of the campsite: the bonfire, and each stick, how toasted its marshmallow is and
    /// where it is, each with its identity, reference and where it stands, but a stick in someone's hand, which only
    /// their presence says.
    pub fn describe(&self) -> Vec<(String, String, String, Option<Vec3>)> {
        let mut objects = vec![(
            "bonfire".to_owned(),
            "campsite/bonfire".to_owned(),
            "the campsite's bonfire, crackling in its ring of stones beside the pool lookout's log seat".to_owned(),
            Some(heart()),
        )];
        objects.extend(self.sticks.iter().enumerate().map(|(index, stick)| {
            let place = match (stick.held, &stick.lying) {
                (Some(_), _) => "held by someone",
                (None, Some(_)) => "lying where it was put down",
                (None, None) => "leaning on the log by the fire",
            };
            (
                format!("stick-{index}"),
                format!("campsite/stick-{index}"),
                format!("a marshmallow on a stick, {}, {place}", looks(stick.toast)),
                stick
                    .held
                    .is_none()
                    .then(|| stick.resting(index).marshmallow()),
            )
        }));
        objects
    }

    /// The details of what it describes as `reference`, for the space's agent: the bonfire, or a stick, how toasted
    /// it is, who holds it, by their number, and where it rests.
    pub fn inspect(&self, reference: &str) -> Option<serde_json::Value> {
        if reference == "campsite/bonfire" {
            return Some(serde_json::json!({
                "kind": "bonfire",
                "at": heart().to_array(),
            }));
        }
        let index: usize = reference.strip_prefix("campsite/stick-")?.parse().ok()?;
        let stick = self.sticks.get(index)?;
        Some(serde_json::json!({
            "kind": "marshmallow stick",
            "toast": stick.toast,
            "looks": looks(stick.toast),
            "held_by": stick.held,
            "on_its_log": stick.held.is_none() && stick.lying.is_none(),
        }))
    }
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
        let turn = stone as f32 / PIT_STONES as f32 * TAU + scatter(stone + 9800) * 0.2;
        let at = FIRE + Vec3::new(turn.sin(), 0.0, turn.cos()) * PIT;
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
        let top: Vec<Vec3> = base
            .iter()
            .map(|corner| {
                let inset = at + (*corner - at) * 0.55;
                Vec3::new(inset.x, lift + height, inset.z)
            })
            .collect();
        let color = colors::STONE * (0.85 + scatter(stone + 9840) * 0.3);
        let middle = at + Vec3::Y * (lift + height * 0.5);
        for side in 0..sides as usize {
            let next = (side + 1) % sides as usize;
            let corners = [base[side], base[next], top[next], top[side]];
            let outward = corners.iter().copied().sum::<Vec3>() / 4.0 - middle;
            facing(shapes, &corners, outward, color * 0.85);
        }
        facing(shapes, &top, Vec3::Y, color);
    }
    // The bonfire: five logs leaning together over the embers, their feet on the ash, charred towards the middle.
    for index in 0..5u32 {
        let turn = index as f32 / 5.0 * TAU + 0.4;
        let way = Vec3::new(turn.sin(), 0.0, turn.cos());
        let foot = FIRE + way * 0.27 + Vec3::Y * 0.03;
        let top = FIRE + way * 0.03 + Vec3::Y * (0.5 + scatter(index + 9850) * 0.08);
        log(
            shapes,
            (foot, top),
            0.04,
            5,
            (palette::BARK * 0.9, colors::CHARRED),
        );
    }
    // Two logs laid across beneath them, burning through.
    for (index, turn) in [0.9f32, 2.6].into_iter().enumerate() {
        let way = Vec3::new(turn.sin(), 0.0, turn.cos());
        let middle = FIRE + Vec3::Y * 0.06;
        log(
            shapes,
            (middle - way * 0.26, middle + way * 0.26),
            0.05,
            5,
            (
                colors::CHARRED * (1.0 + index as f32 * 0.4),
                colors::CHARRED,
            ),
        );
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
        5,
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
        log(&mut marshmallow, span, radius, 7, (colour, colour * 1.08));
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
