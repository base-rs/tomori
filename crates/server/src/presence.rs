// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Where the people in a space are, as their clients last reported, and what that means in the space's own terms: the
//! walls and panels they look and point at, the wall nearest them and the walls in their view, for the space's agent's
//! `presence` tool. It holds no I/O: the hub keeps what each participant's client reports, and asks this what it means.
//! See `design/mcp.md`.
//!
//! The server knows only what clients report: a headset's head and, while tracked, its controllers and hands; a
//! desktop's head. Workers' stations and cards are laid out by each client, so the server cannot tell when someone
//! looks at one; walls, panels and other people's heads it places itself, or is told, so it resolves those.
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tomori_protocol::{Panel, Place, Pose, Projection, Wall, WallKind, wall as layout};

use tomori_environment::sightlines::Solid;

use crate::stage::{Destination, Placement};

/// How long a report stays current: a client in a space reports with every frame, so one not heard from for this long
/// has stopped, such as while its headset sleeps.
pub(crate) const STALE: Duration = Duration::from_secs(5);

/// How far along a ray what it meets is resolved, in metres: a little beyond what the space reaches.
const RANGE: f32 = 30.0;

/// How near a ray passes another person's head to be looking or pointing at them, in metres, as near as it passes a
/// worker to point at one.
const HEAD_RADIUS: f32 = 0.3;

/// Half of a headset's field of view, across and up and down, in degrees: about a Quest 3's.
const VIEW: (f32, f32) = (50.0, 45.0);

/// The size of a panel in a flat wall's slot, and of one floating as the server takes it, in metres, as clients draw
/// it: 318 by 190 font pixels of 1.9 mm.
const PANEL: (f32, f32) = layout::SLOT_SIZE;

/// How far in front of its wall a panel stands, in metres.
const LIFT: f32 = layout::LIFT;

/// What a participant's client last reported of where they are, in the space's scene coordinates.
#[derive(Clone, Debug)]
pub(crate) struct Seen {
    pub head: Pose,
    /// The left and right controllers' aim poses, while tracked.
    pub controllers: [Option<Pose>; 2],
    /// The left and right tracked hands' palms, while tracked.
    pub hands: [Option<[f32; 3]>; 2],
    /// When it was reported.
    pub at: Instant,
    /// The panel they are carrying, by its identity, as their client said.
    pub carrying: Option<String>,
}

impl Seen {
    /// What a presence report says, if its head is a pose: controllers, joints and orientations that are not are left
    /// out, so that nothing a client sends makes the geometry fail.
    pub(crate) fn reported(
        head: Pose,
        left: Option<Pose>,
        right: Option<Pose>,
        hands: Option<&[Option<tomori_protocol::Hand>; 2]>,
        at: Instant,
    ) -> Option<Self> {
        let head = sound(head)?;
        // The palm, the first joint in OpenXR's order, else the wrist.
        let palm = |hand: &Option<tomori_protocol::Hand>| {
            hand.as_ref()
                .filter(|hand| hand.valid())
                .and_then(|hand| hand.joints[0].or(hand.joints[1]))
                .map(|joint| joint.position)
        };
        Some(Self {
            head,
            controllers: [left.and_then(sound), right.and_then(sound)],
            hands: hands.map_or([None, None], |hands| [palm(&hands[0]), palm(&hands[1])]),
            at,
            carrying: None,
        })
    }

    /// It, carrying `panel`, if its identity is within bounds; one beyond them names no panel there is.
    pub(crate) fn carrying(mut self, panel: Option<String>) -> Self {
        self.carrying = panel.filter(|panel| panel.len() <= crate::limits::IDENTIFIER_BYTES);
        self
    }

    /// The panel it says is being carried at `now`, if any: a report gone [`STALE`] says nothing, since its client has
    /// stopped, as when its headset sleeps.
    pub(crate) fn carried(&self, now: Instant) -> Option<&str> {
        self.carrying
            .as_deref()
            .filter(|_| now.saturating_duration_since(self.at) < STALE)
    }

    /// Whether it says anything only a headset reports: a controller or a hand.
    pub(crate) fn tracks_hands(&self) -> bool {
        self.controllers.iter().any(Option::is_some) || self.hands.iter().any(Option::is_some)
    }
}

/// Someone taking part in a space, as the agent names them: their display name, which others may share, their tag,
/// the number that tells them apart, and their head, as they last reported it, if they have. See `design/security.md`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Someone {
    pub name: String,
    pub tag: Option<u32>,
    pub head: Option<Pose>,
}

impl Someone {
    /// Their handle, as the agent names them where their name is not enough: `Ada#23`.
    pub(crate) fn handle(&self) -> String {
        handle(&self.name, self.tag)
    }
}

/// The handle of someone called `name` with `tag`: `Ada#23`, or their name alone without one.
pub(crate) fn handle(name: &str, tag: Option<u32>) -> String {
    match tag {
        Some(tag) => format!("{name}#{tag}"),
        None => name.to_owned(),
    }
}

/// Which of `people` `named` means: the one whose handle it is, `Ada#23`, its name in any case; or else the one whose
/// display name it is, in any case, when only one has it. Or why none is meant: nobody here has it, or several do, when
/// it says which handles name them.
pub(crate) fn whom<'a>(named: &str, people: &'a [Someone]) -> Result<&'a Someone, String> {
    let named = named.trim();
    let key = tomori_protocol::text::name_key;
    if let Some((name, tag)) = named.rsplit_once('#')
        && !tag.is_empty()
        && tag.bytes().all(|byte| byte.is_ascii_digit())
        && let Ok(tag) = tag.parse::<u32>()
        && let Some(person) = people
            .iter()
            .find(|person| person.tag == Some(tag) && key(&person.name) == key(name))
    {
        return Ok(person);
    }
    let called: Vec<&Someone> = people
        .iter()
        .filter(|person| key(&person.name) == key(named))
        .collect();
    // One identity on several devices, sharing a tag, is one person, the first of whose devices joined is meant.
    let mut handles: Vec<String> = Vec::new();
    for person in &called {
        let handle = person.handle();
        if !handles.contains(&handle) {
            handles.push(handle);
        }
    }
    match (called.first(), handles.len()) {
        (None, _) => Err(format!(
            "Nobody called {named:?} in this space has reported where they are."
        )),
        (Some(person), 1) => Ok(person),
        _ => Err(format!(
            "More than one person in this space is called {named:?}: say which, as {}.",
            handles.join(" or ")
        )),
    }
}

/// `pose` with a unit orientation, if it is finite, within reach and has an orientation.
fn sound(pose: Pose) -> Option<Pose> {
    let length = pose
        .orientation
        .iter()
        .map(|value| value * value)
        .sum::<f32>()
        .sqrt();
    (pose
        .position
        .iter()
        .all(|value| value.is_finite() && value.abs() < 10_000.0)
        && length.is_finite()
        && length > 0.5)
        .then(|| Pose {
            position: pose.position,
            orientation: pose.orientation.map(|value| value / length),
        })
}

type Vec3 = [f32; 3];

fn add(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn scale(a: Vec3, by: f32) -> Vec3 {
    a.map(|value| value * by)
}

fn dot(a: Vec3, b: Vec3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn length(a: Vec3) -> f32 {
    dot(a, a).sqrt()
}

/// `vector` turned by the unit quaternion `rotation`.
fn rotate(rotation: [f32; 4], vector: Vec3) -> Vec3 {
    let [x, y, z, w] = rotation;
    let axis = [x, y, z];
    let cross = |a: Vec3, b: Vec3| {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };
    let t = scale(cross(axis, vector), 2.0);
    add(add(vector, scale(t, w)), cross(axis, t))
}

/// The way `pose` faces: its -Z, as a head looks and an OpenXR aim pose points.
fn forward(pose: &Pose) -> Vec3 {
    rotate(pose.orientation, [0.0, 0.0, -1.0])
}

/// The yaw and pitch of `direction`, in degrees: yaw clockwise from -Z as seen from above, so that 90 faces +X, and
/// pitch above the horizontal.
fn angles(direction: Vec3) -> (f32, f32) {
    let yaw = direction[0].atan2(-direction[2]).to_degrees();
    let pitch = direction[1]
        .atan2(direction[0].hypot(direction[2]))
        .to_degrees();
    (yaw, pitch)
}

/// `value` to the nearest centimetre, or degree, as the agent is told it.
fn round(value: f32, places: i32) -> f64 {
    let factor = 10f64.powi(places);
    (f64::from(value) * factor).round() / factor
}

fn position(at: Vec3) -> Value {
    json!(at.map(|axis| round(axis, 2)))
}

/// A rectangle standing in the space: its middle, its right and up directions, unit length, and half its width and
/// height.
#[derive(Clone, Copy, Debug)]
struct Rectangle {
    middle: Vec3,
    right: Vec3,
    up: Vec3,
    half: (f32, f32),
}

impl Rectangle {
    /// The rectangle `size` at `pose`, facing its +Z, moved `offset` in its own frame.
    fn at(pose: &Pose, offset: Vec3, size: (f32, f32)) -> Self {
        Self {
            middle: add(pose.position, rotate(pose.orientation, offset)),
            right: rotate(pose.orientation, [1.0, 0.0, 0.0]),
            up: rotate(pose.orientation, [0.0, 1.0, 0.0]),
            half: (size.0 / 2.0, size.1 / 2.0),
        }
    }

    fn normal(&self) -> Vec3 {
        let (a, b) = (self.right, self.up);
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    }

    /// How far along the ray from `origin` in the unit `direction` it meets the rectangle, from either side.
    fn meets(&self, origin: Vec3, direction: Vec3) -> Option<f32> {
        let normal = self.normal();
        let facing = dot(direction, normal);
        if facing.abs() < 1e-6 {
            return None;
        }
        let along = dot(sub(self.middle, origin), normal) / facing;
        let at = sub(add(origin, scale(direction, along)), self.middle);
        (along > 0.0
            && along <= RANGE
            && dot(at, self.right).abs() <= self.half.0
            && dot(at, self.up).abs() <= self.half.1)
            .then_some(along)
    }

    /// How far `point` is from the rectangle's nearest point.
    fn distance(&self, point: Vec3) -> f32 {
        let offset = sub(point, self.middle);
        let across = dot(offset, self.right).clamp(-self.half.0, self.half.0);
        let up = dot(offset, self.up).clamp(-self.half.1, self.half.1);
        let nearest = add(
            self.middle,
            add(scale(self.right, across), scale(self.up, up)),
        );
        length(sub(point, nearest))
    }
}

/// Where each of `wall`'s slots stands, as clients draw a panel in it, as `tomori_protocol::wall` lays them out: a flat
/// wall's in its plane, its columns and rows numbered from its top left; a curved wall's along its arc, each facing the
/// line the arc curves about.
fn slots(wall: &Wall) -> Vec<Rectangle> {
    // In its frame, tilted as it tilts.
    let standing = wall.frame();
    wall.slot_frames()
        .iter()
        .map(|frame| {
            let pose = frame.pose(&standing);
            Rectangle::at(&pose, [0.0; 3], (frame.size[0], frame.size[1]))
        })
        .collect()
}

/// Where the middle of each of `wall`'s slots is, in the space's scene coordinates, as the server takes it to be when it
/// tells a space's agent what people look at and where walls are: where clients draw a panel in it.
///
/// ```
/// use tomori_protocol::{Pose, Wall, WallKind};
///
/// let wall = Wall::new("wall-1", WallKind::Added, Pose::upright_facing([0.0, 1.5, -2.0], [0.0, 1.5, 0.0]));
/// let slots = tomori_server::presence::slot_centres(&wall);
/// assert_eq!(slots.len(), 6);
/// // The middle slot of the top row is above the wall's middle, just in front of it.
/// assert!((slots[1][1] - 1.707).abs() < 0.01 && slots[1][2] > -2.0);
/// ```
pub fn slot_centres(wall: &Wall) -> Vec<[f32; 3]> {
    slots(wall).iter().map(|slot| slot.middle).collect()
}

/// How far beyond the end of a wall a worker placed by it stands, in metres, from the edge of its last slot: half a
/// worker's sign and a little more, so that its sign stands clear of the wall.
const BEYOND: f32 = 0.9;

/// Where a worker placed by `wall` stands: just beyond the end of it nearer whoever's eyes are at `eyes`, or its right
/// end without them, at its slots' height, facing as it faces, so that it stands by the chart it is reading.
pub(crate) fn beyond(wall: &Wall, eyes: Option<[f32; 3]>) -> Pose {
    let slots = slots(wall);
    let first = slots[0];
    let right = first.right;
    let along = |point: Vec3| dot(sub(point, wall.pose.position), right);
    let (low, high) = slots
        .iter()
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(low, high), slot| {
            let middle = along(slot.middle);
            (
                low.min(middle - slot.half.0),
                high.max(middle + slot.half.0),
            )
        });
    let height = slots.iter().map(|slot| slot.middle[1]).sum::<f32>() / slots.len() as f32;
    let at = |side: f32| {
        let point = add(wall.pose.position, scale(right, side));
        [point[0], height, point[2]]
    };
    let (left_end, right_end) = (at(low - BEYOND), at(high + BEYOND));
    let distance = |point: Vec3, eyes: Vec3| length(sub(point, eyes));
    let end = match eyes {
        Some(eyes) if distance(left_end, eyes) < distance(right_end, eyes) => left_end,
        _ => right_end,
    };
    let facing = rotate(wall.pose.orientation, [0.0, 0.0, 1.0]);
    Pose::upright_facing(end, [end[0] + facing[0], end[1], end[2] + facing[2]])
}

/// How far to someone's right a worker placed beside them stands, in metres, from their eyes, measured level.
const RIGHT_OF: f32 = 1.5;

/// Where a worker placed beside someone whose head is at `head` stands: [`RIGHT_OF`] to their right of the way they
/// face, as the agent stands to their left, a little below their eyes, facing them; placed from their head once, never
/// chasing it.
pub(crate) fn right_of(head: &Pose) -> Pose {
    let ahead = forward(head);
    let (x, z) = match ahead[0].hypot(ahead[2]) {
        level if level > 1e-3 => (ahead[0] / level, ahead[2] / level),
        _ => (0.0, -1.0),
    };
    let [px, py, pz] = head.position;
    let at = [px - z * RIGHT_OF, py - 0.2, pz + x * RIGHT_OF];
    Pose::upright_facing(at, [px, at[1], pz])
}

/// What `wall` of `projection` is called, as the agent is told and may name it: the space's own status wall, `status
/// wall (the Tomori wall)`, as it was called; an environment's wall by the name its environment gave it, if it did, or
/// else, and an added wall, by its number among those of its kind, from 1; and an environment's wall in the status role
/// as both, `environment wall 1 (the status wall)`.
pub(crate) fn wall_name(projection: &Projection, wall: &Wall) -> String {
    let own = own_name(projection, wall);
    match wall.kind {
        WallKind::Environment if wall.status => format!("{own} (the status wall)"),
        _ => own,
    }
}

/// What `wall` of `projection` is called, but for an environment's wall's status role.
fn own_name(projection: &Projection, wall: &Wall) -> String {
    if let (WallKind::Environment, Some(name)) = (&wall.kind, &wall.name) {
        return name.clone();
    }
    let number = projection
        .walls
        .iter()
        .filter(|other| other.kind == wall.kind)
        .position(|other| other.id == wall.id)
        .map_or(1, |index| index + 1);
    match wall.kind {
        WallKind::Tomori => "status wall (the Tomori wall)".to_owned(),
        WallKind::Environment => format!("environment wall {number}"),
        _ => format!("added wall {number}"),
    }
}

/// The names that name the status role, wherever the status wall stands, in any case.
const STATUS_NAMES: [&str; 2] = ["status wall", "Tomori wall"];

/// Whether `named`, without `the`, names `wall` of `projection`: by its identity, its name, or, for an environment's
/// wall in the status role, its own name; and whether it names the status role, as `status wall` or `Tomori wall` do.
fn names(projection: &Projection, wall: &Wall, named: &str) -> Option<bool> {
    let alias = (wall.status || wall.kind == WallKind::Tomori)
        && STATUS_NAMES
            .iter()
            .any(|status| status.eq_ignore_ascii_case(named));
    let own = wall.id.eq_ignore_ascii_case(named)
        || wall_name(projection, wall).eq_ignore_ascii_case(named)
        || own_name(projection, wall).eq_ignore_ascii_case(named);
    (alias || own).then_some(alias || wall.kind == WallKind::Tomori)
}

/// `wall` of `projection` as a sentence names it, after "the": "the status wall", "the environment wall 1" or "the
/// added wall 2", and a wall its environment named by its name in quotes, as "the wall "Details"", which reads as a
/// name rather than a kind of wall.
pub(crate) fn the_wall(projection: &Projection, wall: &Wall) -> String {
    match (&wall.kind, &wall.name) {
        (WallKind::Tomori, _) => "the status wall".to_owned(),
        (WallKind::Environment, Some(name)) => format!("the wall \"{name}\""),
        _ => format!("the {}", wall_name(projection, wall)),
    }
}

/// `words` with its first letter in capitals, to begin a sentence.
pub(crate) fn capitalised(words: &str) -> String {
    let mut letters = words.chars();
    letters.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(letters).collect()
    })
}

/// The parts of `wall` a ray can meet: its slots, and, for a flat wall, the board they stand on.
fn surfaces(wall: &Wall) -> (Vec<Rectangle>, Option<Rectangle>) {
    let board = wall
        .board()
        .map(|size| Rectangle::at(&wall.frame(), [0.0; 3], size));
    (slots(wall), board)
}

/// What a ray meets.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Met {
    /// A panel, by its identity, and the wall and slot it is in, if it is on one.
    Panel {
        panel: String,
        wall: Option<(String, u32)>,
    },
    /// A wall, by its identity, and the empty slot it met, if any.
    Wall { wall: String, slot: Option<u32> },
    /// Another person, by their name and their handle.
    Person { name: String, handle: String },
}

/// What the ray from `origin` in the unit `direction` meets first of `projection`'s walls and panels and `people`'s
/// heads, each with their name and handle, with how far along it, if anything.
pub(crate) fn meet(
    origin: Vec3,
    direction: Vec3,
    projection: &Projection,
    people: &[(&str, &str, Vec3)],
) -> Option<(Met, f32)> {
    let mut met: Option<(Met, f32)> = None;
    let mut consider = |what: Met, along: f32| {
        if met.as_ref().is_none_or(|(_, nearest)| along < *nearest) {
            met = Some((what, along));
        }
    };
    for wall in &projection.walls {
        let (slots, board) = surfaces(wall);
        for (slot, rectangle) in slots.iter().enumerate() {
            let Some(along) = rectangle.meets(origin, direction) else {
                continue;
            };
            let slot = slot as u32;
            let held = projection.panels.iter().find(|panel| {
                matches!(&panel.place, Place::Wall { wall: on, slot: at } if *on == wall.id && *at == slot)
            });
            consider(
                match held {
                    Some(panel) => Met::Panel {
                        panel: panel.id.clone(),
                        wall: Some((wall.id.clone(), slot)),
                    },
                    None => Met::Wall {
                        wall: wall.id.clone(),
                        slot: Some(slot),
                    },
                },
                along,
            );
        }
        // The board behind the slots, a little further than a slot in front of it.
        if let Some(along) = board.and_then(|board| board.meets(origin, direction)) {
            consider(
                Met::Wall {
                    wall: wall.id.clone(),
                    slot: None,
                },
                along + LIFT,
            );
        }
    }
    for panel in &projection.panels {
        if let Place::Free { pose } = &panel.place
            && let Some(along) = Rectangle::at(pose, [0.0; 3], PANEL).meets(origin, direction)
        {
            consider(
                Met::Panel {
                    panel: panel.id.clone(),
                    wall: None,
                },
                along,
            );
        }
    }
    for (name, handle, head) in people {
        let offset = sub(*head, origin);
        let along = dot(offset, direction);
        if along > 0.0
            && along <= RANGE
            && length(sub(offset, scale(direction, along))) <= HEAD_RADIUS
        {
            consider(
                Met::Person {
                    name: (*name).to_owned(),
                    handle: (*handle).to_owned(),
                },
                along,
            );
        }
    }
    met
}

/// The wall of `walls` nearest `point`, and how far its nearest slot, or board, is.
pub(crate) fn nearest_wall(walls: &[Wall], point: Vec3) -> Option<(&Wall, f32)> {
    walls
        .iter()
        .map(|wall| (wall, wall_distance(wall, point)))
        .min_by(|a, b| a.1.total_cmp(&b.1))
}

/// How far `point` is from `wall`'s nearest slot, or board.
fn wall_distance(wall: &Wall, point: Vec3) -> f32 {
    let (slots, board) = surfaces(wall);
    slots
        .iter()
        .chain(board.as_ref())
        .map(|rectangle| rectangle.distance(point))
        .fold(f32::INFINITY, f32::min)
}

/// How far off straight ahead of `head`, across and up, `point` is, in degrees, if it is ahead of it at all.
fn off_ahead(head: &Pose, point: Vec3) -> Option<(f32, f32)> {
    let offset = sub(point, head.position);
    let inverse = {
        let [x, y, z, w] = head.orientation;
        [-x, -y, -z, w]
    };
    let [x, y, z] = rotate(inverse, offset);
    (z < 0.0).then(|| (x.atan2(-z).to_degrees(), y.atan2(-z).to_degrees()))
}

/// The walls of `walls` in the view of someone whose head is at `head`, nearest the middle of it first, each with how
/// far off the middle of the view, in degrees, its nearest slot is: one with a slot within a headset's field of view.
pub(crate) fn walls_in_view<'a>(walls: &'a [Wall], head: &Pose) -> Vec<(&'a Wall, f32)> {
    let mut shown: Vec<(&Wall, f32)> = walls
        .iter()
        .filter_map(|wall| {
            slots(wall)
                .iter()
                .filter_map(|slot| off_ahead(head, slot.middle))
                .filter(|(across, up)| across.abs() <= VIEW.0 && up.abs() <= VIEW.1)
                .map(|(across, up)| across.hypot(up))
                .min_by(f32::total_cmp)
                .map(|off| (wall, off))
        })
        .collect();
    shown.sort_by(|a, b| a.1.total_cmp(&b.1));
    shown
}

/// `met`, as the agent is told it: what kind of thing it is, named as Tomori names it, and how far away.
pub(crate) fn described(met: &Met, along: f32, projection: &Projection) -> Value {
    let wall_named = |id: &str| {
        projection
            .wall(id)
            .map_or_else(|| id.to_owned(), |wall| wall_name(projection, wall))
    };
    let mut described = match met {
        Met::Panel { panel, wall } => {
            let mut described = json!({ "kind": "panel", "panel": panel });
            if let Some(panel) = projection.panel(panel) {
                described["title"] = json!(title(panel));
                if let Some(slug) = &panel.slug {
                    described["id"] = json!(slug);
                }
                if let Some(code) = &panel.code {
                    described["language"] = json!(code.language);
                }
            }
            match wall {
                Some((wall, slot)) => {
                    described["wall"] = json!(wall);
                    described["wall_name"] = json!(wall_named(wall));
                    described["slot"] = json!(slot);
                }
                None => described["floating"] = json!(true),
            }
            described
        }
        Met::Wall { wall, slot } => {
            json!({ "kind": "wall", "wall": wall, "wall_name": wall_named(wall), "slot": slot })
        }
        Met::Person { name, handle } => json!({ "kind": "person", "name": name, "id": handle }),
    };
    described["distance"] = json!(round(along, 2));
    described
}

/// `described`, a panel's description, with the id the agent gave `panel`, if it gave one.
fn named(mut described: Value, panel: &Panel) -> Value {
    if let Some(slug) = &panel.slug {
        described["id"] = json!(slug);
    }
    described
}

/// What a panel is called: its title, or the start of its body, on one line of at most [`TITLE_CHARS`] characters, as
/// text others choose is quoted, since whoever wrote it may not be whoever asked the agent. See `design/mcp.md`.
pub(crate) fn title(panel: &Panel) -> String {
    let named = if panel.title.is_empty() {
        panel.body.lines().next().unwrap_or_default()
    } else {
        &panel.title
    };
    crate::space::one_line(named, TITLE_CHARS)
}

/// The most characters of a panel's title the agent is told.
const TITLE_CHARS: usize = 80;

/// A pose, as the agent is told it: where it is, and the way it faces as a yaw and pitch.
fn facing(pose: &Pose) -> Value {
    let (yaw, pitch) = angles(forward(pose));
    json!({ "position": position(pose.position), "yaw": round(yaw, 0), "pitch": round(pitch, 0) })
}

/// What one person in a space is, as the agent's `presence` tool reports them.
pub(crate) struct Person<'a> {
    pub name: &'a str,
    /// The number that tells them apart from others of the same name.
    pub tag: Option<u32>,
    pub role: tomori_protocol::ConnectionRole,
    pub engagement: Option<tomori_protocol::Engagement>,
    pub seen: Option<&'a Seen>,
    /// Whether their current connection has reported a controller or a hand, as only a headset does.
    pub headset: bool,
}

/// `person`, as the `presence` tool reports them at `now`, in `projection`, among `people`'s heads.
pub(crate) fn report(
    person: &Person<'_>,
    projection: &Projection,
    people: &[(&str, &str, Vec3)],
    now: Instant,
) -> Value {
    let role = serde_json::to_value(person.role).unwrap_or(Value::Null);
    let handle = handle(person.name, person.tag);
    let mut report = json!({
        "name": person.name,
        "id": handle,
        "role": role,
        "engagement": person.engagement,
    });
    if person.role == tomori_protocol::ConnectionRole::Visitor {
        report["device"] = Value::Null;
        report["presence"] = json!("Visitors send no presence, so where they are is not known.");
        return report;
    }
    let Some(seen) = person.seen else {
        report["device"] = Value::Null;
        report["presence"] = json!("Their client has not reported where they are.");
        return report;
    };
    let ago = now.saturating_duration_since(seen.at);
    report["device"] = json!(if person.headset { "headset" } else { "desktop" });
    report["updated_seconds_ago"] = json!(round(ago.as_secs_f32(), 1));
    report["stale"] = json!(ago >= STALE);
    report["head"] = facing(&seen.head);
    // The others, but not themselves, whom they may look or point at.
    let others: Vec<(&str, &str, Vec3)> = people
        .iter()
        .filter(|(_, other, _)| *other != handle)
        .copied()
        .collect();
    let looking = meet(seen.head.position, forward(&seen.head), projection, &others);
    report["looking_at"] = looking.map_or(Value::Null, |(met, along)| {
        described(&met, along, projection)
    });
    if person.headset {
        for (side, index) in [("left", 0), ("right", 1)] {
            report[format!("{side}_hand")] = match (seen.controllers[index], seen.hands[index]) {
                (Some(controller), _) => {
                    let mut hand = facing(&controller);
                    hand["tracked"] = json!("controller");
                    hand
                }
                (None, Some(palm)) => json!({ "position": position(palm), "tracked": "hand" }),
                (None, None) => Value::Null,
            };
        }
        report["pointing_at"] = seen.controllers[1]
            .and_then(|ray| meet(ray.position, forward(&ray), projection, &others))
            .map_or(Value::Null, |(met, along)| {
                described(&met, along, projection)
            });
    }
    report["nearest_wall"] = nearest_wall(&projection.walls, seen.head.position).map_or(
        Value::Null,
        |(wall, distance)| {
            json!({ "wall": wall.id, "wall_name": wall_name(projection, wall), "distance": round(distance, 2) })
        },
    );
    report["walls_in_view"] = json!(
        walls_in_view(&projection.walls, &seen.head)
            .iter()
            .map(|(wall, _)| wall.id.as_str())
            .collect::<Vec<_>>()
    );
    report
}

/// `projection`'s walls, and the panels floating in it, as the `presence` tool describes them, so that the agent can
/// reason about where to put what it shows: each wall's identity, name, kind, where it stands, faces and tilts, its
/// size, how large its panels' content is drawn within its slots and against a standard slot's, whether it is locked,
/// its version, and which of its slots hold which panels and which are free.
pub(crate) fn walls(projection: &Projection) -> (Value, Value) {
    let walls = projection
        .walls
        .iter()
        .map(|wall| {
            let held: Vec<(u32, &Panel)> = projection
                .panels
                .iter()
                .filter_map(|panel| match &panel.place {
                    Place::Wall { wall: on, slot } if *on == wall.id => Some((*slot, panel)),
                    _ => None,
                })
                .collect();
            let used: Vec<Value> = (0..wall.slots())
                .filter_map(|slot| {
                    held.iter().find(|(at, _)| *at == slot).map(|(_, panel)| {
                        named(
                            json!({
                                "slot": slot,
                                "panel": panel.id,
                                "title": title(panel),
                                "code": panel.code.is_some(),
                                "pinned": panel.pinned,
                            }),
                            panel,
                        )
                    })
                })
                .collect();
            let free: Vec<u32> = (0..wall.slots())
                .filter(|slot| !held.iter().any(|(at, _)| at == slot))
                .collect();
            // A wall's face looks along its +Z.
            let (yaw, _) = angles(rotate(wall.pose.orientation, [0.0, 0.0, 1.0]));
            let tilt = wall.tilt();
            // Its size along its face, curved or not, and its grid and slots' size, which the agent may change.
            let shape = wall
                .radius()
                .map_or(json!("flat"), |radius| json!({ "arc": round(radius, 2) }));
            let size = wall.size();
            let grid = wall.grid();
            let kind = wall.kind;
            // The walls it is snapped to, by name, across which of its edges.
            let joined: Vec<Value> = wall
                .joined
                .iter()
                .filter_map(|join| projection.wall(&join.wall).map(|other| (join, other)))
                .map(|(join, other)| {
                    json!({
                        "edge": serde_json::to_value(join.edge).unwrap_or(Value::Null),
                        "wall": other.id,
                        "name": wall_name(projection, other),
                    })
                })
                .collect();
            let mut described = json!({
                "id": wall.id,
                "name": wall_name(projection, wall),
                "kind": serde_json::to_value(kind).unwrap_or(Value::Null),
                "shape": shape,
                "position": position(wall.pose.position),
                "facing_yaw": round(yaw, 0),
                "tilt": round(tilt, 0),
                "size": [round(size.0, 2), round(size.1, 2)],
                "columns": grid.columns,
                "rows": grid.rows,
                "slot_size": [round(wall.slot_size().0, 2), round(wall.slot_size().1, 2)],
                "content_scale": round(wall.content_scale(), 2),
                "text_scale": round(wall.text_scale(), 2),
                "locked": wall.is_locked(),
                "version": wall.version,
                "used": used,
                "free": free,
            });
            if let Some(group) = &wall.group {
                described["group"] = json!(group);
                described["joined"] = json!(joined);
            }
            described
        })
        .collect::<Vec<_>>();
    let floating = projection
        .panels
        .iter()
        .filter_map(|panel| match &panel.place {
            Place::Free { pose } => Some(named(
                json!({
                    "panel": panel.id,
                    "title": title(panel),
                    "position": position(pose.position),
                    "pinned": panel.pinned,
                }),
                panel,
            )),
            _ => None,
        })
        .collect::<Vec<_>>();
    (json!(walls), json!(floating))
}

/// How far in front of someone a panel the agent floats in front of them stands, in metres, and how far below their
/// eyes: within reach, and low enough to read without hiding what is behind it.
const FRONT: (f32, f32) = (1.2, 0.15);

/// Where the agent's panel goes when it asks for none, for whoever asked, whose head was at `asker` as they began
/// speaking: onto the wall they were looking at, if they could read it from there; or else floating beside them, clear
/// of the environment's `solids`; or onto the Tomori wall when nobody asked. See `design/projection.md`.
pub(crate) fn by_default(
    asker: Option<&Pose>,
    projection: &Projection,
    solids: &[Solid],
) -> Destination {
    asker.map_or(Destination::Tomori, |head| {
        looked_at(head, projection).unwrap_or_else(|| beside(head, solids))
    })
}

/// Floating beside someone whose head is at `head`, clear of `solids`, as [`Destination::Beside`] places it.
fn beside(head: &Pose, solids: &[Solid]) -> Destination {
    Destination::Beside {
        head: *head,
        solids: solids.to_vec(),
    }
}

/// How far from someone's eyes a wall may be, in metres, for the agent's panel to be put on it for them by where they
/// look: within it, a flat wall's text, at its 1.9 mm font pixel, is about as sharp as the curved Tomori wall's, at
/// seven tenths of that, from home, about 2.6 m away, which is as far as walls are designed to be read from.
/// Comfortable reading, an x-height of about 0.45° (see `design/text.md`), would need a slot's text within about 1.4 m,
/// nearer than any wall stands from home, so the cap follows the walls' own design rather than that. Beyond it, as on
/// a wall 7 m away, a slot's text is too small to read without pulling the panel off. See `design/projection.md`.
pub(crate) const READING: f32 = 4.0;

/// How far from someone's eyes `wall` may be for the agent's panel to be put on it for them by where they look:
/// [`READING`], or as many times further as its slots' text, as its content scale draws it, is larger than a standard
/// slot's, since its text is as legible from that far, so that a wall five times as large is read from 20 m, and at
/// half its content scale from 10 m. A wall of smaller text keeps [`READING`], since walls stand nearer than it from
/// home. See `design/projection.md`.
pub(crate) fn reading(wall: &Wall) -> f32 {
    READING * wall.text_scale().max(1.0)
}

/// How far off the ray from someone's eyes a wall's slot may be, in degrees, for them to be looking at the wall: a head
/// faces a little above what its eyes look at, and passes above a low wall, such as the Tomori wall below the swarm,
/// that they plainly look towards.
pub(crate) const GAZE: f32 = 10.0;

/// The angle between the unit `ahead` and `toward`, in degrees.
fn angle(ahead: Vec3, toward: Vec3) -> f32 {
    (dot(ahead, toward) / length(toward).max(1e-6))
        .clamp(-1.0, 1.0)
        .acos()
        .to_degrees()
}

/// The walls of `projection` within reading of `eyes`, each with how far its nearest slot, or board, is.
fn readable(projection: &Projection, eyes: Vec3) -> Vec<(&Wall, f32)> {
    projection
        .walls
        .iter()
        .map(|wall| (wall, wall_distance(wall, eyes)))
        .filter(|(wall, distance)| *distance <= reading(wall))
        .collect()
}

/// The nearest of `walls`, by its identity.
fn nearest_of<'a>(walls: impl Iterator<Item = &'a (&'a Wall, f32)>) -> Option<String> {
    walls
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(wall, _)| wall.id.clone())
}

/// Where someone whose head is at `head` is looking, for a panel, if at a wall within [`READING`] of their eyes: the
/// nearest of those they look at, which are the wall the ray from their eyes meets, in the slot it meets if that is
/// empty, and those with a slot within [`GAZE`] of the ray. A person in the way is looked past.
fn looked_at(head: &Pose, projection: &Projection) -> Option<Destination> {
    let (eyes, ahead) = (head.position, forward(head));
    let met = match meet(eyes, ahead, projection, &[]) {
        Some((Met::Wall { wall, slot }, _)) => Some((wall, slot)),
        Some((
            Met::Panel {
                wall: Some((wall, _)),
                ..
            },
            _,
        )) => Some((wall, None)),
        _ => None,
    };
    let readable = readable(projection, eyes);
    // The walls looked at: the one met, and those with a slot near the ray.
    let wall = nearest_of(readable.iter().filter(|(wall, _)| {
        met.as_ref().is_some_and(|(id, _)| *id == wall.id)
            || slots(wall)
                .iter()
                .any(|slot| angle(ahead, sub(slot.middle, eyes)) <= GAZE)
    }))?;
    let slot = met.and_then(|(id, slot)| slot.filter(|_| id == wall));
    Some(Destination::Wall { wall, slot })
}

/// Where `looking` puts a panel for someone whose head is at `head`: onto the wall they look at, as [`looked_at`]
/// finds it; else the nearest wall within [`READING`] in their view; else the nearest wall within it; else, with no
/// wall within reading, floating beside them, clear of `solids`.
fn looking_at(head: &Pose, projection: &Projection, solids: &[Solid]) -> Destination {
    if let Some(looked) = looked_at(head, projection) {
        return looked;
    }
    let eyes = head.position;
    let readable = readable(projection, eyes);
    let in_view = walls_in_view(&projection.walls, head);
    let shown = nearest_of(
        readable
            .iter()
            .filter(|(wall, _)| in_view.iter().any(|(seen, _)| seen.id == wall.id)),
    );
    match shown {
        Some(wall) => Destination::Wall { wall, slot: None },
        None if !readable.is_empty() => Destination::Nearest(eyes),
        None => beside(head, solids),
    }
}

/// Floating in front of someone whose head is at `head`, facing them, level, the way they face about the vertical, or,
/// looking straight up or down, the way the top of their head faces.
fn in_front(head: &Pose) -> Destination {
    Destination::Free(front_of(head))
}

/// The pose floating in front of someone whose head is at `head`, as [`in_front`] places a panel.
pub(crate) fn front_of(head: &Pose) -> Pose {
    ahead_of(head, FRONT)
}

/// Where `count` panels the agent floats beside someone whose head is at `head` stand, placed one after another as the
/// server places a reply's, among `walls`, clear of them and of `solids`, the environment's scenery as boxes from least
/// to greatest corner in the space's coordinates, or, with no spot clear, in front of them. Clients draw a panel where
/// the server sends it; this says where that is, for previews of how it looks. See `design/projection.md`.
///
/// ```
/// use tomori_protocol::Pose;
///
/// // Looking along −Z, as a head looks along its −Z.
/// let head = Pose::at([0.0, 1.6, 0.0]).facing_towards([0.0, 1.6, 1.0]);
/// let fanned = tomori_server::presence::fanned_beside(&head, &[], &[], 3);
/// // To their right, to their left, then further out to their right.
/// assert!(fanned[0].position[0] > 0.5 && fanned[1].position[0] < -0.5);
/// assert!(fanned[2].position[0] > fanned[0].position[0]);
/// ```
pub fn fanned_beside(
    head: &Pose,
    walls: &[Wall],
    solids: &[([f32; 3], [f32; 3])],
    count: usize,
) -> Vec<Pose> {
    let solids: Vec<Solid> = solids
        .iter()
        .map(|(low, high)| Solid::new(*low, *high))
        .collect();
    let mut placed = Vec::new();
    for _ in 0..count {
        let pose = crate::stage::beside_spot(head, walls, &placed, &solids)
            .unwrap_or_else(|| front_of(head));
        placed.push(pose);
    }
    placed
}

/// How far in front of someone a wall the agent adds for them stands, in metres, and how far below their eyes its
/// middle is: as a wall a participant adds from the hand menu stands.
const WALL_FRONT: (f32, f32) = (1.8, 0.1);

/// Where a wall the agent adds for someone whose head is at `head` stands, its slots `scale` times a standard slot's and
/// `height` metres tall: [`WALL_FRONT`] ahead of them, level, a little below their eyes, upright and facing them. A wall
/// larger than [`BIG_SCALE`] stands as many times further as it is larger, so that its text looks as large as that
/// one's does there, rather than a big wall towering over them; and a wall so tall that it would reach below the floor
/// where people arrive, for someone standing above it, stands raised until its bottom is [`WALL_CLEARANCE`] above it.
pub(crate) fn wall_in_front(head: &Pose, scale: f32, height: f32) -> Pose {
    let (distance, drop) = WALL_FRONT;
    let mut pose = ahead_of(head, (distance * (scale / BIG_SCALE).max(1.0), drop));
    let least = height / 2.0 + WALL_CLEARANCE;
    if head.position[1] > 0.0 && pose.position[1] < least {
        pose.position[1] = least;
    }
    pose
}

/// How far above the floor where people arrive, in metres, a wall the agent adds stands at least.
const WALL_CLEARANCE: f32 = 0.1;

/// The largest a wall's slots are, against a standard slot's, that stands at [`WALL_FRONT`] when added: twice, the
/// largest walls were before they could be big.
const BIG_SCALE: f32 = 2.0;

/// The pose `distance` ahead of someone whose head is at `head`, level with where they face, `drop` below their eyes,
/// upright and facing them; looking straight down or up, ahead is the way the top of their head faces, or its back.
fn ahead_of(head: &Pose, (distance, drop): (f32, f32)) -> Pose {
    let ahead = forward(head);
    let top = rotate(head.orientation, [0.0, 1.0, 0.0]);
    let level = |[x, _, z]: Vec3| x.hypot(z);
    let (x, z) = if level(ahead) > 1e-3 {
        (ahead[0] / level(ahead), ahead[2] / level(ahead))
    } else {
        let toward = if ahead[1] > 0.0 { -1.0 } else { 1.0 };
        let length = level(top).max(1e-6);
        (toward * top[0] / length, toward * top[2] / length)
    };
    let [px, py, pz] = head.position;
    let at = [px + x * distance, py - drop, pz + z * distance];
    Pose::upright_facing(at, [px, py - drop, pz])
}

/// The farthest a layout of walls the agent composes stands from whoever it is for, in metres, and the nearer distances
/// it is tried at when its slots would be too far to read, nearest last.
const LAYOUT_DISTANCES: (f32, [f32; 4]) = (3.5, [2.4, 2.0, 1.6, 1.2]);

/// How wide a layout is to be seen, as the tangent of half the angle it takes from the eyes: about 60°, so that it is
/// seen whole by moving the eyes, rather than the head.
const LAYOUT_VIEW: f32 = 0.577;

/// The lowest a layout's middle stands, in metres, and how far above someone's eyes it may stand.
const LAYOUT_HEIGHTS: (f32, f32) = (0.8, 0.6);

/// Where the middle of a layout of walls the agent composes for someone whose head is at `head` stands, facing them,
/// upright, at each distance it is tried at: level with where they face, a little below their eyes, or, `looking`, as
/// high as they look, within [`LAYOUT_HEIGHTS`]. A flat layout `width` metres wide stands first as far as it takes to be
/// seen whole, within [`LAYOUT_VIEW`], at least 1.5 m; a curved one at its `radius`, so that its arc curves about them;
/// either no further than [`LAYOUT_DISTANCES`]'s farthest, and then nearer, at each of its nearer distances.
pub(crate) fn layout_middles(
    head: &Pose,
    looking: bool,
    width: f32,
    radius: Option<f32>,
) -> Vec<Pose> {
    let ahead = forward(head);
    let level = ahead_of(head, (1.0, 0.0));
    let [px, py, pz] = head.position;
    let (x, z) = (level.position[0] - px, level.position[2] - pz);
    let slope = if looking {
        ahead[1] / ahead[0].hypot(ahead[2]).max(1e-3)
    } else {
        0.0
    };
    let first = radius
        .unwrap_or((width / 2.0 / LAYOUT_VIEW).max(1.5))
        .min(LAYOUT_DISTANCES.0);
    std::iter::once(first)
        .chain(
            LAYOUT_DISTANCES
                .1
                .into_iter()
                .filter(|distance| *distance < first),
        )
        .map(|distance| {
            let height = if looking {
                py + slope * distance
            } else {
                py - WALL_FRONT.1
            };
            let height = height.min(py + LAYOUT_HEIGHTS.1).max(LAYOUT_HEIGHTS.0);
            let at = [px + x * distance, height, pz + z * distance];
            Pose::upright_facing(at, [px, height, pz])
        })
        .collect()
}

/// The wall `named` means in `projection`, by its identity: a wall by its identity or name, such as `Tomori wall` or
/// `added wall 2`, with or without `wall` or `the` before it; or the wall someone is looking at or is nearest, in the
/// forms `looking [NAME]` and `near [NAME]`, of whoever asked, whose head was at `asker`, or of one of `people`. Or why
/// none is meant, as for `front`, which names no wall.
pub(crate) fn which_wall(
    named: &str,
    asker: Option<&Pose>,
    people: &[Someone],
    projection: &Projection,
) -> Result<String, String> {
    let by_name = Placement::Wall(named.to_owned(), None);
    if let Ok(Destination::Wall { wall, .. }) =
        destination(&by_name, asker, people, projection, &[])
    {
        // The status role is the wall in it.
        return Ok(projection
            .walls
            .iter()
            .find(|shown| shown.id == wall || wall == tomori_protocol::TOMORI_WALL && shown.status)
            .map_or(wall, |shown| shown.id.clone()));
    }
    let place = Placement::parse(named).ok_or_else(|| {
        format!(
            "There is no wall called {:?} here: name one as presence does, or say looking or near, with a name or for whoever asked.",
            crate::space::one_line(named, 80)
        )
    })?;
    let (name, looking) = match place {
        Placement::Near(name) => (name, false),
        Placement::Looking(name) => (name, true),
        Placement::Front(_) => {
            return Err(
                "front names no wall: say which wall, or looking or near someone.".to_owned(),
            );
        }
        Placement::Beside(_) => {
            return Err(
                "beside names no wall: say which wall, or looking or near someone.".to_owned(),
            );
        }
        // Named after `wall`, it is a wall by its identity or name, which no wall here has.
        Placement::Wall(named, _) => {
            return Err(format!("There is no wall called {named:?} here."));
        }
        Placement::Anchor(_) => {
            return Err(
                "anchor names no wall: say which wall, or looking or near someone.".to_owned(),
            );
        }
    };
    let head = head_of(name.as_deref(), asker, people)?;
    // The wall they are looking at, if they are, or else the one nearest them; there is always the status wall.
    let seen = looking.then(|| looking_at(&head, projection, &[]));
    Ok(match seen {
        Some(Destination::Wall { wall, .. }) => wall,
        _ => nearest_wall(&projection.walls, head.position)
            .map_or(tomori_protocol::TOMORI_WALL.to_owned(), |(wall, _)| {
                wall.id.clone()
            }),
    })
}

/// Where the head of the participant `name`, of `people`, is, as they last reported, or, naming nobody, of whoever
/// asked, whose head was at `asker` as they began speaking; or why not.
pub(crate) fn head_of(
    name: Option<&str>,
    asker: Option<&Pose>,
    people: &[Someone],
) -> Result<Pose, String> {
    match name {
        Some(name) => whom(name, people)?.head.ok_or_else(|| {
            format!(
                "Nobody called {:?} in this space has reported where they are.",
                name.trim()
            )
        }),
        None => asker.copied().ok_or_else(|| {
            "Nobody asked for this as they spoke, so name whom it is for.".to_owned()
        }),
    }
}

/// Where `place` puts the agent's panel, in `projection`, clear of the environment's `solids`, for whoever asked, whose
/// head was at `asker` as they began speaking, among `people`, the participants in the space, by name or handle, and
/// their heads as they last reported; or why it cannot, as when it names someone who has not reported, a name several
/// have, or a wall that is not here. A form that names nobody is about whoever asked.
pub(crate) fn destination(
    place: &Placement,
    asker: Option<&Pose>,
    people: &[Someone],
    projection: &Projection,
    solids: &[Solid],
) -> Result<Destination, String> {
    let whose = |name: &Option<String>| head_of(name.as_deref(), asker, people);
    Ok(match place {
        Placement::Near(name) => Destination::Nearest(whose(name)?.position),
        Placement::Looking(name) => looking_at(&whose(name)?, projection, solids),
        Placement::Front(name) => in_front(&whose(name)?),
        Placement::Beside(name) => beside(&whose(name)?, solids),
        // The hub finds a slot at an anchor, as only it knows the space's anchors.
        Placement::Anchor(name) => {
            return Err(format!("There is no anchor called {name:?} here."));
        }
        Placement::Wall(named, slot) => {
            let named = named.trim();
            let named = named
                .get(..4)
                .filter(|the| the.eq_ignore_ascii_case("the "))
                .map_or(named, |_| &named[4..]);
            let (wall, role) = projection
                .walls
                .iter()
                .find_map(|wall| names(projection, wall, named).map(|role| (wall, role)))
                .ok_or_else(|| format!("There is no wall called {named:?} here."))?;
            // A slot it does not have is refused, rather than taken as its most prominent free one.
            if let Some(slot) = slot.filter(|slot| *slot >= wall.slots()) {
                return Err(format!(
                    "{} has no slot {slot}: its slots are numbered from 0 to {}.",
                    capitalised(&the_wall(projection, wall)),
                    wall.slots() - 1
                ));
            }
            // Named as the status wall, a panel goes there as a status panel, wherever that stands.
            Destination::Wall {
                wall: if role {
                    tomori_protocol::TOMORI_WALL.to_owned()
                } else {
                    wall.id.clone()
                },
                slot: *slot,
            }
        }
    })
}

/// Where the agent stands when it goes to someone whose head is at `head`: `tomori_protocol::entourage::SUMMONED` in
/// front of them, the way they face about the vertical, a little above their eyes, as a participant's summons stands
/// it, with room for its wall beneath it however they sit or stand (`entourage::summoned`), facing them. See
/// `design/environments.md`.
pub(crate) fn before(head: &Pose) -> Pose {
    let ahead = forward(head);
    let (x, z) = match ahead[0].hypot(ahead[2]) {
        level if level > 1e-3 => (ahead[0] / level, ahead[2] / level),
        _ => (0.0, -1.0),
    };
    let [px, py, pz] = head.position;
    let away = tomori_protocol::entourage::SUMMONED;
    tomori_protocol::entourage::summoned(
        [px + x * away, py, pz + z * away],
        head.position,
        tomori_protocol::entourage::FLOOR,
    )
}

/// What the `presence` and `move_agent` tools say of the agent beyond where it stands: how it moves now, whom it talks
/// with, who pinned it and when it goes home. See `design/mcp.md`.
#[derive(Clone, Debug, Default)]
pub(crate) struct AgentState {
    /// Where it was summoned to, or none at home.
    pub summoned: Option<Pose>,
    /// Where it has reached, while it is on its way somewhere, or home.
    pub at: Option<Pose>,
    pub pinned: bool,
    /// Who pinned it, by their handle, where a participant did.
    pub pinned_by: Option<String>,
    /// Whom it talks with now, or last did, by their handle.
    pub partner: Option<String>,
    /// How long until it goes home if nothing more is said, while it follows.
    pub returns_in: Option<std::time::Duration>,
    pub movement: tomori_protocol::AgentMovement,
    /// The anchor it stands at, if any.
    pub anchor: Option<String>,
}

/// Where the agent is, as the `presence` and `move_agent` tools say: where it stands, or is going, which way it faces,
/// whether its place is home, and where that is, and, on its way, where it has reached; its mode, at home, going home, following the speaker, pinned or away where it was
/// asked to go, and whether it is on its way; whom it talks with, who pinned it and, while it follows, how long until
/// it goes home if nothing more is said; how the space lets it move, and the anchor it stands at. See
/// `design/environments.md`.
pub(crate) fn agent(anchors: &tomori_protocol::Anchors, state: &AgentState) -> Value {
    let home = crate::hub::agent_home(anchors);
    let at = state.summoned.unwrap_or(home);
    // The way it faces, its +Z, clockwise from −Z.
    let facing = rotate(at.orientation, [0.0, 0.0, 1.0]);
    let mode = match (state.pinned, state.summoned, state.movement.mode) {
        (true, _, _) => "pinned",
        (false, None, _) if state.at.is_some() => "going home",
        (false, None, _) => "home",
        (false, Some(_), tomori_protocol::Movement::Follow) => "following",
        (false, Some(_), _) => "away",
    };
    let movement = match state.movement.mode {
        tomori_protocol::Movement::Follow => "follows the speaker",
        tomori_protocol::Movement::Home => "stays home",
        _ => "moves only when asked",
    };
    let mut agent = json!({
        "position": position(at.position),
        "facing_yaw": round(angles(facing).0, 0),
        "at_home": state.summoned.is_none(),
        "home": position(home.position),
        "mode": mode,
        "movement": movement,
    });
    if let Some(now) = state.at {
        agent["moving"] = json!(true);
        agent["now_at"] = position(now.position);
    }
    if let Some(partner) = &state.partner {
        agent["partner"] = json!(partner);
    }
    if let Some(by) = state.pinned_by.as_ref().filter(|_| state.pinned) {
        agent["pinned_by"] = json!(by);
    }
    if let Some(returns) = state.returns_in {
        agent["returns_home_in_s"] = json!(returns.as_secs());
    }
    if let Some(anchor) = &state.anchor {
        agent["anchor"] = json!(anchor);
    }
    agent
}

/// The space's cluster anchors, `home` first, as the `presence` tool tells its agent of them, so that it finds the
/// place it means by its kind and label rather than by coordinates: each one's name, kind and label, where its point
/// is and the way it faces, the room its cluster takes, whether the environment placed it as it runs, the id the agent
/// may inspect the object it stands by by, among the objects of `environment`, and what stands there, in the order of
/// their slots: the workers, by session and title, those gathered into its "+N more" sign saying so, and whoever
/// `placed_by` says placed each, where a participant did; the panels, by identity; and the agent. See `design/mcp.md`.
pub(crate) fn anchors(
    anchors: &[tomori_protocol::cluster::Anchor],
    seating: &crate::seating::Seating,
    workers: &[tomori_protocol::Worker],
    environment: &str,
    placed_by: &dyn Fn(&str) -> Option<String>,
) -> Value {
    use crate::seating::Member;
    let listed: Vec<Value> = anchors
        .iter()
        .map(|anchor| {
            let gathered = seating.gathered(anchor);
            let mut members: Vec<&(Member, usize)> = seating.members(&anchor.name).iter().collect();
            members.sort_by_key(|(_, slot)| *slot);
            let mut standing = Vec::new();
            let mut panels = Vec::new();
            let mut agent = false;
            for (member, _) in members {
                match member {
                    Member::Worker(session) => {
                        let title = workers
                            .iter()
                            .find(|worker| &worker.id == session)
                            .map_or("", |worker| worker.title.as_str());
                        let mut listed = json!({
                            "session": session,
                            "title": crate::space::one_line(title, 160),
                        });
                        if gathered.contains(&member) {
                            listed["gathered"] = json!(true);
                        }
                        if let Some(by) = placed_by(session) {
                            listed["placed_by"] = json!(by);
                        }
                        standing.push(listed);
                    }
                    Member::Panel(panel) => panels.push(json!(panel)),
                    Member::Agent => agent = true,
                }
            }
            let facing = rotate(anchor.pose.orientation, [0.0, 0.0, 1.0]);
            let mut listed = json!({
                "name": anchor.name,
                "position": position(anchor.pose.position),
                "facing_yaw": round(angles(facing).0, 0),
                "workers": standing,
            });
            if !panels.is_empty() {
                listed["panels"] = json!(panels);
            }
            if agent {
                listed["agent"] = json!(true);
            }
            if let Some(kind) = &anchor.kind {
                listed["kind"] = json!(kind);
            }
            if let Some(label) = &anchor.label {
                listed["label"] = json!(label);
            }
            listed["extent"] = match anchor
                .extent
                .or((!anchor.is_home()).then_some(tomori_protocol::cluster::ROOM))
            {
                Some(extent) => json!({
                    "width": round(extent.width, 2),
                    "height": round(extent.height, 2),
                    "depth": round(extent.depth, 2),
                }),
                None => json!("the arc about home"),
            };
            if anchor.placed {
                listed["placed"] = json!(true);
            }
            if let Some(object) = &anchor.object {
                listed["object"] = json!(crate::space::inspectable(environment, object));
            }
            listed
        })
        .collect();
    json!(listed)
}

/// How the `presence` tool's numbers are to be read.
pub(crate) const COORDINATES: &str = "Positions are in the space's scene coordinates, in metres: y is up, and home, where everyone arrives, wherever the environment has them arrive, is at the origin, facing -z. The agent stands at its own home, which the environment chooses, unless it went to someone or was called elsewhere. Yaw is in degrees clockwise from -z as seen from above, so 90 faces +x; pitch is in degrees above the horizontal. A wall's facing_yaw is the way its face looks. Panels hold their wall's slots, numbered from its top left as seen from in front, left to right then down.";

#[cfg(test)]
mod tests;
