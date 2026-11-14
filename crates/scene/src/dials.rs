// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The space's timers, drawn as small dials at the agent's home, beside its workers: each a clock face of cubes, as a
//! worker is a flock of them, its sector the time left to its next firing, and a sign above it naming it, its schedule
//! and condition in a few words and its countdown, worked out here from when it next fires and this client's clock, so
//! that it ticks without the server saying anything more. They stand where the home's layout leaves room, above the
//! agent where it can, or else beneath it ([`places`]). Each can be pointed at, as a worker can. See
//! `design/timers.md`.
use std::time::Instant;

use glam::{Mat3, Quat, Vec3};
use tomori_protocol::{Timer, TimerState};

use crate::anchors::Anchor;
use crate::delegation::Cube;
use crate::pointing::{Ray, Shape};
use crate::sides::{Hands, Side};

/// The most timers a space keeps, and so the most dials drawn: the rest, from a server that sends more, are not.
pub const MOST: usize = 8;
/// The most dials in a row above the agent: a fifth stands in a second row above the first.
pub const PER_ROW: usize = 4;
/// How far apart, along the arc about whoever watches, the dials of a row stand, in metres at their own size: a little
/// wider than a dial's sign.
pub const APART: f32 = 0.65;
/// How far above a row of dials the next stands, in metres at their own size: clear above the first row's signs.
pub const ROW_APART: f32 = 0.6;
/// How far from its middle a dial's face reaches, in metres at its own size.
pub const RADIUS: f32 = 0.1;
/// How far above its dial's face a dial's sign's bottom edge stands, how wide the sign is at most, and how tall, in
/// metres at the dial's own size, as `tomori-render` draws it: what a dial and its sign take, with [`RADIUS`].
pub const SIGN_LIFT: f32 = 0.1;
pub const SIGN_WIDTH: f32 = 0.61;
pub const SIGN_HEIGHT: f32 = 0.22;
/// How far the swarm reaches about its middle, across, up and down, and deep, in metres: its halo, which stands 1.13 m
/// to either side and 0.7 m above and below it, and its agents, which reach 0.64 m above and below it as it speaks.
pub const SWARM_REACH: Vec3 = Vec3::new(1.2, 0.75, 0.5);
/// How far clear of the agent's reach, above or below it, the dials and their signs stand, in metres.
pub const CLEAR: f32 = 0.05;
/// How far from whoever watches a dial stands at its own size, in metres, as a worker's sign does; beyond it, a dial,
/// its sign and the room between them grow in proportion, so that they read as at that distance.
pub const REACH: f32 = 3.2;
/// How near a dial's middle the ray must pass to point at it, in metres at its own size, within
/// [`crate::delegation::POINTING_REACH`]; beyond it, growing with the distance, as a worker's does, but smaller than a
/// worker's, since dials stand closer.
pub const POINTING_RADIUS: f32 = 0.2;
/// The most characters of a timer's name a dial shows, as a server keeps it.
pub const NAME_CHARS: usize = 40;
/// The most characters of a timer's schedule, and of its condition, a dial shows.
pub const WORDS_CHARS: usize = 80;
/// How long a dial pulses for, once its timer has just fired, in seconds.
pub const PULSE_SECONDS: f32 = 1.2;
/// How long a dial takes to grow in as its timer is set, and to shrink away as it is cancelled or ends, in seconds.
const ARRIVING_SECONDS: f32 = 0.35;
const LEAVING_SECONDS: f32 = 0.5;
/// The marks round a dial's face, one for each minute of an hour.
const SEGMENTS: usize = 60;
/// How much paler and larger the dial pointed at is drawn, as a worker pointed at is.
const HIGHLIGHT_WHITENING: f32 = 0.45;
const HIGHLIGHT_SCALE: f32 = 1.25;
/// How far to either side of the agent, along the arc, a row's dials may stand, in metres, larger in proportion where
/// home stands further than [`REACH`] from the audience, where something stands in the way of a row centred on it.
pub const ALONG: f32 = 3.0;
/// How much the agent's reach must shrink, in metres, before the dials are placed for the smaller reach: an animated
/// avatar's bounds change as it moves, and the dials keep their places through that.
pub const SETTLE: f32 = 0.25;
/// How far this client's clock may drift from the steady clock's reckoning, in seconds, before it is read again.
pub const DRIFT: f64 = 1.0;

/// A dial as it is drawn this frame: the timer's name, its schedule and its condition in a few words each, as `every
/// hour` and `when present`, what its pill says, its state, where its middle stands, the eyes it faces, how large it is
/// drawn as a share of its own size, how opaque it is as it grows in or shrinks away, and whether it is pointed at.
#[derive(Clone, Debug, PartialEq)]
pub struct Dial {
    pub name: String,
    pub schedule: String,
    pub condition: String,
    pub pill: String,
    pub state: TimerState,
    pub position: [f32; 3],
    pub facing: [f32; 3],
    pub scale: f32,
    pub opacity: f32,
    pub pointed: bool,
}

impl Dial {
    /// Whether it is within the bounds this client draws.
    pub fn valid(&self) -> bool {
        let finite = |point: &[f32; 3]| point.iter().all(|v| v.is_finite() && v.abs() <= 100.0);
        !self.name.is_empty()
            && self.name.chars().count() <= NAME_CHARS
            && self.schedule.chars().count() <= WORDS_CHARS
            && self.condition.chars().count() <= WORDS_CHARS
            && self.pill.chars().count() <= 24
            && finite(&self.position)
            && finite(&self.facing)
            && self.scale.is_finite()
            && (0.3..=10.0).contains(&self.scale)
            && self.opacity.is_finite()
            && (0.0..=1.0).contains(&self.opacity)
    }
}

/// This client's clock, in seconds since the Unix epoch, as the dials count down by it: reckoned from the steady clock
/// between frames, so that it ticks smoothly, and read again from the system's whenever the two drift more than
/// [`DRIFT`] apart, forward as after the device slept, while the steady clock stood still, or back as the system's clock
/// is corrected. See `design/timers.md`.
#[derive(Clone, Copy, Debug, Default)]
pub struct Clock {
    base: Option<(f64, Instant)>,
}

impl Clock {
    /// The time at `at`, given the system's clock reads `system` then.
    pub fn now(&mut self, system: f64, at: Instant) -> f64 {
        let reckoned = self
            .base
            .map(|(unix, then)| match at.checked_duration_since(then) {
                Some(since) => unix + since.as_secs_f64(),
                None => unix - then.duration_since(at).as_secs_f64(),
            });
        match reckoned {
            Some(reckoned) if (reckoned - system).abs() <= DRIFT => reckoned,
            _ => {
                self.base = Some((system, at));
                system
            }
        }
    }
}

/// How long until `next`, in seconds since the Unix epoch, from `now`, as a dial's pill says it: minutes and seconds
/// within the hour, as `12:05`; hours and minutes within the day, as `3h 05m`; then days and hours, as `2d 3h`;
/// `due now` once it is due, as while it waits for the agent's turn; and nothing when it has no next firing.
pub fn countdown(next: Option<u64>, now: f64) -> Option<String> {
    let left = left(next?, now);
    if left == 0 {
        return Some("due now".into());
    }
    Some(match left {
        0..3600 => format!("{}:{:02}", left / 60, left % 60),
        3600..86_400 => format!("{}h {:02}m", left / 3600, left % 3600 / 60),
        _ => format!("{}d {}h", left / 86_400, left % 86_400 / 3600),
    })
}

/// The whole seconds until `next` from `now`, rounded up, so that it is 0 only once the firing is due.
fn left(next: u64, now: f64) -> u64 {
    let left = (next as f64 - now).ceil();
    if !left.is_finite() || left <= 0.0 {
        0
    } else {
        left.min(1e9) as u64
    }
}

/// What a dial's pill says of `timer` at `now`: its countdown while it waits; `Paused` and its countdown while nobody is
/// present for it; `On arrival` while a firing waits for someone to arrive; `Fired` for the minute after it fires.
pub fn pill(timer: &Timer, now: f64) -> String {
    let left = countdown(timer.next, now);
    match timer.state {
        TimerState::Paused => match left {
            Some(left) => format!("Paused {left}"),
            None => "Paused".into(),
        },
        TimerState::Awaiting => "On arrival".into(),
        TimerState::Fired => "Fired".into(),
        TimerState::Waiting | TimerState::Unknown => left.unwrap_or_else(|| "—".into()),
    }
}

/// The colour of a dial's lit marks in `state`, which tints its sign's pill too: a distinct colour for each, with a
/// distinct shape besides (see [`Dials`]), so that colour is never all that tells them apart.
pub fn color(state: TimerState) -> [f32; 3] {
    match state {
        TimerState::Waiting => [0.35, 0.8, 1.0],
        TimerState::Paused => [0.55, 0.58, 0.65],
        TimerState::Awaiting => [1.0, 0.72, 0.24],
        TimerState::Fired => [0.3, 0.95, 0.65],
        TimerState::Unknown => [0.7, 0.65, 0.8],
    }
}

/// What stands near the agent's home, which the dials keep clear of: a vertical cylinder about `centre`, `radius`
/// across, from `low` to `high`, as a worker and its sign take, turned any way, or a dial and its sign.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Extent {
    pub centre: Vec3,
    pub radius: f32,
    pub low: f32,
    pub high: f32,
}

impl Extent {
    /// What a worker standing at `position`, its sign drawn `scale` of its full size as a compact cluster draws it,
    /// takes, facing whoever's eyes are at `audience`: its flock, and its sign, a metre across, drawn larger beyond 3.2 m
    /// from them, as `tomori-render` draws it, 0.3 m above it.
    pub fn worker(position: Vec3, scale: f32, audience: Vec3) -> Self {
        let far = ((position.x - audience.x).hypot(position.z - audience.z) / REACH).max(1.0);
        Self {
            centre: position,
            radius: 0.55 * scale * far,
            low: position.y - 0.2 * scale,
            high: position.y + 0.7 * scale * far,
        }
    }

    /// What a dial at `position`, drawn `scale` of its own size, takes with its sign above it.
    pub fn dial(position: Vec3, scale: f32) -> Self {
        Self {
            centre: position,
            radius: SIGN_WIDTH / 2.0 * scale,
            low: position.y - RADIUS * scale,
            high: position.y + (RADIUS + SIGN_LIFT + SIGN_HEIGHT) * scale,
        }
    }

    /// What the slots of `walls` take, each a panel's room, as the dials keep clear of: the space's status wall, wherever
    /// it stands, among them.
    pub fn walls(walls: &[tomori_protocol::Wall]) -> Vec<Self> {
        let mut extents = Vec::new();
        for wall in walls {
            let (width, height) = wall.slot_size();
            extents.extend(
                (0..wall.slots())
                    .filter_map(|slot| wall.slot_pose(slot))
                    .map(|pose| {
                        let centre = Vec3::from_array(pose.position);
                        Self {
                            centre,
                            radius: width / 2.0,
                            low: centre.y - height / 2.0,
                            high: centre.y + height / 2.0,
                        }
                    }),
            );
        }
        extents
    }

    /// Whether it and `other` take any of the same room, within `margin`.
    pub fn overlaps(&self, other: &Self, margin: f32) -> bool {
        let across = (self.centre.x - other.centre.x).hypot(self.centre.z - other.centre.z);
        across < self.radius + other.radius + margin
            && self.low < other.high + margin
            && other.low < self.high + margin
    }
}

/// Where the dials may stand: about the agent's `home`, facing whoever's eyes are at `audience`, clear of the agent's
/// reach about its middle, `agent`, across, up and down, and deep, of each of `extents`, what the home's layout stands
/// there, at its every step, of each of `walls`, the slots of the walls the space shows, its status wall among them,
/// and between `floor` and `ceiling`, if the layout has one, as a ring does.
#[derive(Clone, Debug, PartialEq)]
pub struct Room {
    pub home: Anchor,
    pub audience: Vec3,
    pub agent: Vec3,
    pub floor: f32,
    pub ceiling: Option<f32>,
    pub extents: Vec<Extent>,
    pub walls: Vec<Extent>,
}

impl Room {
    /// The room about `home`, facing `audience`, the swarm's reach about it, nothing else there, no ceiling, and the
    /// lowest a worker stands.
    pub fn open(home: Anchor, audience: Vec3) -> Self {
        Self {
            home,
            audience,
            agent: SWARM_REACH,
            floor: tomori_protocol::cluster::FLOOR + tomori_protocol::cluster::LOWEST,
            ceiling: None,
            extents: Vec::new(),
            walls: Vec::new(),
        }
    }

    /// Whether a dial and its sign at `at`, `scale` of its own size, stands clear of everything in it but other dials,
    /// between its floor and ceiling.
    fn fits(&self, at: Vec3, scale: f32) -> bool {
        let dial = Extent::dial(at, scale);
        dial.low >= self.floor - 1e-4
            && self
                .ceiling
                .is_none_or(|ceiling| dial.high <= ceiling + 1e-4)
            && self
                .extents
                .iter()
                .chain(&self.walls)
                .all(|extent| !dial.overlaps(extent, 0.05))
    }

    /// Whether dials and their signs at `places` stand clear of everything in it, and of one another, between its floor
    /// and ceiling.
    pub fn clear(&self, places: &[(Vec3, f32)]) -> bool {
        places.iter().enumerate().all(|(index, (at, scale))| {
            let dial = Extent::dial(*at, *scale);
            self.fits(*at, *scale)
                && places[..index]
                    .iter()
                    .all(|(other, scale)| !dial.overlaps(&Extent::dial(*other, *scale), 0.0))
        })
    }
}

/// The ways the dials may stand, in the order tried: above the agent in rows of four at their own size; above it in one
/// row at 0.7; beneath it in one row at their own size, then at 0.7, then at half; and above it in one row at half.
const WAYS: [(bool, usize, f32); 6] = [
    (true, PER_ROW, 1.0),
    (true, MOST, 0.7),
    (false, MOST, 1.0),
    (false, MOST, 0.7),
    (false, MOST, 0.5),
    (true, MOST, 0.5),
];

/// Where each of `count` dials stands in `room`, at most [`MOST`], in the order their timers were set, with how large
/// each is drawn as a share of its own size: in the first of [`WAYS`] that stands every dial and its sign clear of
/// everything in `room`, or else as the first does. Above the agent, its rows stand from [`CLEAR`] above its reach,
/// [`ROW_APART`] apart; beneath it, its row stands with its signs [`CLEAR`] below its reach. Each row is centred on the
/// agent along the arc about the audience through it, as far from them as the agent, its dials [`APART`] apart, or,
/// where something stands in the way of that, such as a status wall beneath the agent, each as near the agent as it
/// stands clear, half that apart at a time, within [`ALONG`] of it, left to right; all of it larger in proportion where
/// home stands further than [`REACH`] from the audience. Where the audience stands at home, the rows run across the way
/// home faces.
pub fn places(count: usize, room: &Room) -> Vec<(Vec3, f32)> {
    let count = count.min(MOST);
    if count == 0 {
        return Vec::new();
    }
    WAYS.iter()
        .find_map(|&(above, per_row, size)| stand(count, room, above, per_row, size))
        .unwrap_or_else(|| centred(count, room, true, PER_ROW, 1.0))
}

/// `count` dials in `room`, `above` the agent or beneath it, in rows of `per_row`, `size` of their own size, centred,
/// or else each as near the middle of its row as it stands clear; or `None` where they do not all stand clear.
fn stand(count: usize, room: &Room, above: bool, per_row: usize, size: f32) -> Option<Places> {
    let centred = centred(count, room, above, per_row, size);
    if room.clear(&centred) {
        return Some(centred);
    }
    let scale = centred[0].1;
    let step = APART * scale / 2.0;
    let mut places: Places = Vec::with_capacity(count);
    let rows = count.div_ceil(per_row);
    for row in 0..rows {
        let height = centred[row * per_row].0.y;
        let start = places.len();
        let wanted = (count - start).min(per_row);
        // From the middle, out to either side in turn, half the dials' spacing at a time.
        for k in 0.. {
            let along = (k + 1) / 2 * if k % 2 == 1 { 1 } else { -1 };
            let offset = along as f32 * step;
            if offset.abs() > ALONG * scale / size + 1e-4 || places.len() - start == wanted {
                break;
            }
            let ground = along_arc(room, offset);
            let at = Vec3::new(ground.x, height, ground.z);
            let dial = Extent::dial(at, scale);
            if room.fits(at, scale)
                && places
                    .iter()
                    .all(|(other, scale)| !dial.overlaps(&Extent::dial(*other, *scale), 0.0))
            {
                places.push((at, scale));
            }
        }
        if places.len() - start < wanted {
            return None;
        }
    }
    // Row by row, each left to right as the audience sees it.
    let right = along_arc(room, 1.0) - along_arc(room, 0.0);
    places.sort_by(|a, b| {
        a.0.y
            .total_cmp(&b.0.y)
            .then(a.0.dot(right).total_cmp(&b.0.dot(right)))
    });
    Some(places)
}

/// The point on the ground `along` metres along the arc about the audience through the agent's home, to their right as
/// they see it, or across the way home faces where they stand at home.
fn along_arc(room: &Room, along: f32) -> Vec3 {
    let centre = room.home.position;
    let away = Vec3::new(centre.x - room.audience.x, 0.0, centre.z - room.audience.z);
    let distance = away.length();
    if distance >= 0.3 {
        // Clockwise as they see it for a point to their right.
        let turn = Quat::from_rotation_y(-along / distance);
        Vec3::new(room.audience.x, 0.0, room.audience.z) + turn * away
    } else {
        let facing = room.home.orientation * Vec3::Z;
        let right = Vec3::new(-facing.z, 0.0, facing.x);
        Vec3::new(centre.x, 0.0, centre.z) + right * along
    }
}

/// `count` dials in `room`, `above` the agent or beneath it, in rows of `per_row`, `size` of their own size, each row
/// centred on the agent.
fn centred(count: usize, room: &Room, above: bool, per_row: usize, size: f32) -> Places {
    let centre = room.home.position;
    let distance = Vec3::new(centre.x - room.audience.x, 0.0, centre.z - room.audience.z).length();
    let scale = (distance / REACH).max(1.0) * size;
    let first = if above {
        centre.y + room.agent.y + CLEAR + RADIUS * scale
    } else {
        centre.y - room.agent.y - CLEAR - (RADIUS + SIGN_LIFT + SIGN_HEIGHT) * scale
    };
    (0..count)
        .map(|index| {
            let row = index / per_row;
            let in_row = (count - row * per_row).min(per_row);
            let column = (index % per_row) as f32 - (in_row as f32 - 1.0) / 2.0;
            let at = along_arc(room, column * APART * scale);
            (
                Vec3::new(at.x, first + row as f32 * ROW_APART * scale, at.z),
                scale,
            )
        })
        .collect()
}

/// Where each dial stands, and how large it is drawn as a share of its own size.
type Places = Vec<(Vec3, f32)>;

/// A dial shown, by its timer's name: the timer as last sent, where it is placed and stands now, how large it is drawn,
/// how long it has stood and has been leaving, how long it has pulsed for, the whole seconds its pill last counted, and
/// whether its face counts a minute, as for a firing within a minute of when the dial first saw it.
struct Shown {
    timer: Timer,
    minute: bool,
    place: Vec3,
    at: Vec3,
    scale: f32,
    age: f32,
    leaving: Option<f32>,
    pulse: Option<f32>,
    counted: Option<u64>,
}

/// The space's dials: placed as its timers come and go, each gliding to its place as others leave, growing in and
/// shrinking away, pulsing once as its timer fires, drawn as cubes beside the workers' and pointed at. See
/// `design/timers.md`.
#[derive(Default)]
pub struct Dials {
    shown: Vec<Shown>,
    /// Drawn alongside [`Self::shown`], their words made again only as they change.
    dials: Vec<Dial>,
    /// The timer each hand's ray points at, by name, and where it meets its dial.
    pointed: Hands<Option<String>>,
    hit: Hands<Option<Vec3>>,
    /// Whether the space's state was current as of the last update.
    current: bool,
    /// The places last stood for how many dials, in what room.
    placed: Option<(usize, Room, Places)>,
}

impl Dials {
    /// Forget every dial, as when the space is left or another joined, or this client reconnects.
    pub fn clear(&mut self) {
        self.shown.clear();
        self.dials.clear();
        self.pointed = Hands::default();
        self.hit = Hands::default();
        self.placed = None;
        self.current = false;
    }

    /// The dials drawn this frame, the live ones in the order their timers were set, then those leaving.
    pub fn dials(&self) -> &[Dial] {
        &self.dials
    }

    /// The timer whose dial the last [`Self::point`] met, by name.
    pub fn pointed(&self) -> Option<&str> {
        self.pointed_by(Side::Right)
    }

    /// The timer whose dial `side`'s ray last met, by name.
    pub fn pointed_by(&self, side: Side) -> Option<&str> {
        self.pointed[side].as_deref()
    }

    /// Point at the dials with `ray`, or with nothing: the nearest live dial it meets, a sphere about its middle of
    /// [`POINTING_RADIUS`] growing with its distance beyond [`crate::delegation::POINTING_REACH`], as a worker's does,
    /// and how far along the ray it is met, where the ray passes nearest its middle. A leaving dial is not pointed at.
    pub fn point(&mut self, ray: Option<Ray>) -> Option<(f32, Vec3)> {
        self.point_with(Side::Right, ray)
    }

    /// Point at the dials with `side`'s ray, `ray`, as [`Self::point`] does with the right's; a dial either ray meets is
    /// drawn pointed at.
    pub fn point_with(&mut self, side: Side, ray: Option<Ray>) -> Option<(f32, Vec3)> {
        let mut hit: Option<(f32, usize, Vec3)> = None;
        if let Some(ray) = ray {
            for (index, shown) in self.shown.iter().enumerate() {
                let radius = crate::delegation::pointing_radius(ray.origin, shown.at)
                    * (POINTING_RADIUS / crate::delegation::POINTING_RADIUS)
                    * shown.scale;
                let sphere = Shape::Sphere {
                    center: shown.at,
                    radius,
                };
                if shown.leaving.is_none()
                    && let Some(distance) = sphere.intersect(&ray)
                    && hit.is_none_or(|(nearest, ..)| distance < nearest)
                {
                    hit = Some((distance, index, shown.at));
                }
            }
        }
        let name = hit.map(|(_, index, _)| self.shown[index].timer.name.as_str());
        if self.pointed[side].as_deref() != name {
            self.pointed[side] = name.map(str::to_owned);
        }
        self.hit[side] = ray.zip(hit).map(|(ray, (distance, _, centre))| {
            ray.at((centre - ray.origin).dot(ray.direction).max(distance))
        });
        hit.zip(self.hit[side])
            .map(|((distance, ..), point)| (distance, point))
    }

    /// Show `timers`, the first [`MOST`] of them, as the space's state says them, `current` or not, as while this client
    /// reconnects, a name once, standing in `room`, at `now`, in
    /// seconds since the Unix epoch, `dt` seconds after the last update, drawing each dial's cubes into `cubes`. Nothing
    /// is allocated from frame to frame but as timers, their words or the room change, or a countdown ticks.
    pub fn update(
        &mut self,
        timers: &[Timer],
        current: bool,
        room: &Room,
        now: f64,
        dt: f32,
        cubes: &mut Vec<Cube>,
    ) {
        let dt = if dt.is_finite() {
            dt.clamp(0.0, 0.05)
        } else {
            0.0
        };
        // The live timers, by their index in `timers`.
        let mut live = [0; MOST];
        let mut count = 0;
        for (index, timer) in timers.iter().enumerate() {
            if count < MOST
                && !timer.name.is_empty()
                && live[..count]
                    .iter()
                    .all(|other| timers[*other].name != timer.name)
            {
                live[count] = index;
                count += 1;
            }
        }
        let live = &live[..count];
        let stale = self
            .placed
            .as_ref()
            .is_none_or(|(placed, placed_in, _)| *placed != count || placed_in != room);
        if stale {
            self.placed = Some((count, room.clone(), places(count, room)));
        }
        let mut places = [(Vec3::ZERO, 1.0); MOST];
        let placed = &self.placed.as_ref().expect("placed").2;
        places[..placed.len()].copy_from_slice(placed);
        let places = &places[..placed.len()];
        for shown in &mut self.shown {
            shown.leaving.get_or_insert(0.0);
        }
        for (&index, &(place, scale)) in live.iter().zip(places) {
            let timer = &timers[index];
            let name = clipped(&timer.name);
            let found = self.shown.iter().position(|shown| shown.timer.name == name);
            match found {
                Some(found) => {
                    let shown = &mut self.shown[found];
                    // A firing pulses once, for a client that saw its timer otherwise just before, since it last joined:
                    // not for one first seen just fired, as on joining or reconnecting within the minute.
                    // Nor for a firing it learns of only as its state is current again.
                    if timer.state == TimerState::Fired
                        && shown.timer.state != TimerState::Fired
                        && current
                        && self.current
                    {
                        shown.pulse = Some(0.0);
                    }
                    shown.leaving = None;
                    shown.place = place;
                    shown.scale = scale;
                    if !same(&shown.timer, timer) {
                        if shown.timer.next != timer.next {
                            shown.minute = within_a_minute(timer.next, now);
                        }
                        shown.timer = timer.clone();
                        shown.timer.name = name.to_owned();
                        shown.counted = None;
                    }
                }
                None => {
                    let mut timer = timer.clone();
                    timer.name = name.to_owned();
                    self.shown.push(Shown {
                        pulse: None,
                        minute: within_a_minute(timer.next, now),
                        timer,
                        place,
                        at: place,
                        scale,
                        age: 0.0,
                        leaving: None,
                        counted: None,
                    });
                }
            }
        }
        for shown in &mut self.shown {
            shown.age += dt;
            if let Some(leaving) = &mut shown.leaving {
                *leaving += dt;
            }
            if let Some(pulse) = &mut shown.pulse {
                *pulse += dt;
            }
            shown.pulse = shown.pulse.filter(|pulse| *pulse < PULSE_SECONDS);
            shown.at = glide(shown.at, shown.place, dt);
        }
        let before = self.shown.len();
        self.shown.retain(|shown| {
            shown
                .leaving
                .is_none_or(|leaving| leaving < LEAVING_SECONDS)
        });
        // Those leaving after the live ones, which stay in the order their timers were set.
        let order = |shown: &Shown| {
            (
                shown.leaving.is_some(),
                live.iter()
                    .position(|index| clipped(&timers[*index].name) == shown.timer.name)
                    .unwrap_or(MOST),
            )
        };
        let sorted = self.shown.is_sorted_by(|a, b| order(a) <= order(b));
        if !sorted {
            self.shown.sort_by_key(order);
        }
        if !sorted || before != self.shown.len() || self.dials.len() != self.shown.len() {
            self.dials.clear();
        }
        let audience = room.audience;
        for (index, shown) in self.shown.iter_mut().enumerate() {
            let opacity = smooth((shown.age / ARRIVING_SECONDS).min(1.0))
                * (1.0 - smooth((shown.leaving.unwrap_or(0.0) / LEAVING_SECONDS).min(1.0)));
            let pointed = shown.leaving.is_none()
                && self
                    .pointed
                    .any(|_, pointed| pointed.as_deref() == Some(&shown.timer.name));
            draw(shown, audience, room.home, now, opacity, pointed, cubes);
            let counted = shown.timer.next.map(|next| left(next, now));
            let made = index < self.dials.len() && shown.counted == counted;
            if index >= self.dials.len() {
                self.dials.push(Dial {
                    name: shown.timer.name.clone(),
                    schedule: shown.timer.schedule.chars().take(WORDS_CHARS).collect(),
                    condition: shown.timer.condition.chars().take(WORDS_CHARS).collect(),
                    pill: pill(&shown.timer, now),
                    state: shown.timer.state,
                    position: [0.0; 3],
                    facing: [0.0; 3],
                    scale: 1.0,
                    opacity,
                    pointed,
                });
            }
            let dial = &mut self.dials[index];
            if !made {
                // Its words again, only as they or its countdown change.
                renew(&mut dial.schedule, &shown.timer.schedule);
                renew(&mut dial.condition, &shown.timer.condition);
                dial.pill = pill(&shown.timer, now);
                dial.state = shown.timer.state;
                shown.counted = counted;
            }
            dial.position = shown.at.to_array();
            dial.facing = audience.to_array();
            dial.scale = shown.scale.clamp(0.3, 10.0);
            dial.opacity = opacity;
            dial.pointed = pointed;
        }
        self.current = current;
    }

    /// Where the last [`Self::point`] met the dial it points at.
    pub fn hit_point(&self) -> Option<Vec3> {
        self.hit.right
    }
}

/// Make `words` `text` cut to [`WORDS_CHARS`], if it is not already.
fn renew(words: &mut String, text: &str) {
    if !words.chars().eq(text.chars().take(WORDS_CHARS)) {
        *words = text.chars().take(WORDS_CHARS).collect();
    }
}

/// `name`, cut to [`NAME_CHARS`].
fn clipped(name: &str) -> &str {
    name.char_indices()
        .nth(NAME_CHARS)
        .map_or(name, |(end, _)| &name[..end])
}

/// Whether `shown`, a timer as a dial shows it, its name cut, is `timer` as sent.
fn same(shown: &Timer, timer: &Timer) -> bool {
    shown.name == clipped(&timer.name)
        && shown.schedule == timer.schedule
        && shown.condition == timer.condition
        && shown.next == timer.next
        && shown.state == timer.state
}

/// How much of a dial's face its sector fills at `now` for a firing at `next`: the minutes left of an hour, full from an
/// hour on, or with nothing to count down to, so that it fills smoothly across the hour; or, where its face counts a
/// `minute`, the seconds left of a minute, so that a timer set seconds ahead drains across the whole face.
pub fn fill(next: Option<u64>, now: f64, minute: bool) -> f32 {
    let Some(next) = next else { return 1.0 };
    let span = if minute { 60.0 } else { 3600.0 };
    ((next as f64 - now).max(0.0) / span).min(1.0) as f32
}

/// Whether a firing at `next` is within a minute of `now`, so that a dial first seeing it so counts a minute on its
/// face, as for a timer set seconds ahead, rather than the sliver of an hour it would be. A dial seeing a longer one
/// keeps counting the hour, so that its face never jumps as the last minute begins.
pub fn within_a_minute(next: Option<u64>, now: f64) -> bool {
    next.is_some_and(|next| next as f64 - now <= 60.0)
}

fn smooth(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

/// Where a dial at `from` stands after `dt` seconds gliding to `to`, as a worker glides.
fn glide(from: Vec3, to: Vec3, dt: f32) -> Vec3 {
    let next = from + (to - from) * (1.0 - (-4.0 * dt).exp());
    if next.distance(to) < 1e-4 { to } else { next }
}

/// How large a dial pulsing for `elapsed` seconds is drawn, as a share of its size: swelling by a third and back.
fn swell(elapsed: f32) -> f32 {
    1.0 + 0.35 * (std::f32::consts::PI * (elapsed / PULSE_SECONDS).clamp(0.0, 1.0)).sin()
}

/// Draw `shown`'s dial into `cubes`, facing `audience`, at `now`, `opacity` grown in, paler and larger if `pointed`: a
/// dark plate behind sixty marks round its face, every fifth longer, those of its sector lit in its state's colour and
/// the rest dim; a hand from its hub to the end of its sector; while it waits, a mote going round its rim once a minute
/// as the seconds count down, unless its face counts a minute, its hand counting them; while paused, two bars across its hub and no hand; while it waits for an arrival, its hub
/// breathing and no hand; and just fired, the whole face lit, swelling once.
fn draw(
    shown: &Shown,
    audience: Vec3,
    home: Anchor,
    now: f64,
    opacity: f32,
    pointed: bool,
    cubes: &mut Vec<Cube>,
) {
    let centre = shown.at;
    let normal = Vec3::new(audience.x - centre.x, 0.0, audience.z - centre.z)
        .try_normalize()
        .unwrap_or(home.orientation * Vec3::Z);
    let right = Vec3::new(normal.z, 0.0, -normal.x);
    let facing = Quat::from_mat3(&Mat3::from_cols(right, Vec3::Y, normal));
    let state = shown.timer.state;
    let mut size = shown.scale * opacity.max(0.0);
    let mut lit = Vec3::from_array(color(state));
    if let Some(pulse) = shown.pulse {
        size *= swell(pulse);
        lit = lit.lerp(Vec3::ONE, 0.5 * (1.0 - pulse / PULSE_SECONDS));
    }
    if pointed {
        size *= HIGHLIGHT_SCALE;
        lit = lit.lerp(Vec3::ONE, HIGHLIGHT_WHITENING);
    }
    let dim = lit * 0.22 + Vec3::splat(0.03);
    let radius = RADIUS * size;
    let local = |x: f32, y: f32, z: f32| centre + right * x + Vec3::Y * y + normal * z;
    let mut push = |position: Vec3, scale: Vec3, color: Vec3, orientation: Quat| {
        if cubes.len() >= crate::delegation::MAX_CUBES || scale.min_element() <= 0.00001 {
            return;
        }
        let cube = Cube {
            position: position.to_array(),
            scale: scale.min(Vec3::ONE).to_array(),
            color: color.clamp(Vec3::ZERO, Vec3::ONE).to_array(),
            orientation: orientation.normalize().to_array(),
        };
        if cube.valid() {
            cubes.push(cube);
        }
    };
    // The plate: nine squares turned a tenth of a right angle apart, together nearly round, as a sign's plate is dark.
    for turn in 0..9 {
        push(
            local(0.0, 0.0, -0.012 * size),
            Vec3::new(radius * 1.75, radius * 1.75, 0.004 * size),
            Vec3::new(0.016, 0.03, 0.055),
            facing * Quat::from_rotation_z(turn as f32 * std::f32::consts::FRAC_PI_2 / 9.0),
        );
    }
    let fill = match state {
        TimerState::Fired | TimerState::Awaiting => 1.0,
        _ => fill(shown.timer.next, now, shown.minute),
    };
    let marks = fill * SEGMENTS as f32;
    for index in 0..SEGMENTS {
        // Clockwise from the top, as whoever watches sees it.
        let angle = index as f32 * std::f32::consts::TAU / SEGMENTS as f32;
        let long = index % 5 == 0;
        let length = if long { 0.24 } else { 0.14 } * radius;
        let across = (marks - index as f32).clamp(0.0, 1.0);
        let color = dim.lerp(lit, across);
        let out = radius - length / 2.0;
        push(
            local(angle.sin() * out, angle.cos() * out, 0.0),
            Vec3::new(
                if long { 0.09 } else { 0.06 } * radius,
                length,
                0.05 * radius,
            ),
            color,
            facing * Quat::from_rotation_z(-angle),
        );
    }
    let hub = 0.16 * radius;
    match state {
        TimerState::Paused => {
            for side in [-1.0, 1.0] {
                push(
                    local(side * 0.17 * radius, 0.0, 0.01 * size),
                    Vec3::new(0.16 * radius, 0.55 * radius, 0.06 * radius),
                    lit,
                    facing,
                );
            }
        }
        TimerState::Awaiting => {
            let breath = 1.0 + 0.4 * ((now * 1.6).sin() as f32 * 0.5 + 0.5);
            push(
                local(0.0, 0.0, 0.0),
                Vec3::splat(hub * 1.6 * breath),
                lit,
                facing,
            );
        }
        _ => {
            push(local(0.0, 0.0, 0.0), Vec3::splat(hub), lit, facing);
            let angle = fill * std::f32::consts::TAU;
            let length = 0.72 * radius;
            push(
                local(
                    angle.sin() * length / 2.0,
                    angle.cos() * length / 2.0,
                    0.01 * size,
                ),
                Vec3::new(0.07 * radius, length, 0.05 * radius),
                lit,
                facing * Quat::from_rotation_z(-angle),
            );
            // A face counting a minute has its hand count the seconds, without a mote.
            if matches!(state, TimerState::Waiting | TimerState::Unknown)
                && !shown.minute
                && let Some(next) = shown.timer.next
            {
                let seconds = (next as f64 - now).max(0.0).rem_euclid(60.0) as f32;
                let angle = seconds / 60.0 * std::f32::consts::TAU;
                let out = radius * 0.62;
                push(
                    local(angle.sin() * out, angle.cos() * out, 0.01 * size),
                    Vec3::splat(0.1 * radius),
                    Vec3::ONE.lerp(lit, 0.3),
                    facing * Quat::from_rotation_z(-angle),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests;
