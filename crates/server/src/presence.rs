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
use tomori_protocol::{Panel, Place, Pose, Projection, WALL_SLOTS, Wall, WallKind};

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

/// The size of a panel in a flat wall's slot, in metres, as clients draw it: 318 by 190 font pixels of 1.9 mm.
const PANEL: (f32, f32) = (0.6042, 0.361);

/// The size of a flat wall, in metres: 1,050 by 462 font pixels of 1.9 mm.
const FLAT_WALL: (f32, f32) = (1.995, 0.8778);

/// Where the middle of each column and row of a flat wall's slots is, from its middle, in metres.
const COLUMNS: [f32; 3] = [-0.6498, 0.0, 0.6498];
const ROWS: [f32; 2] = [0.2071, -0.1995];

/// How much smaller a curved wall's slots are than a flat wall's, and how far apart, in metres.
const ARC_SCALE: f32 = 0.7;
const ARC_GAP: f32 = 0.03;

/// How far in front of its wall a panel stands, in metres.
const LIFT: f32 = 0.01;

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
        })
    }

    /// Whether it says anything only a headset reports: a controller or a hand.
    pub(crate) fn tracks_hands(&self) -> bool {
        self.controllers.iter().any(Option::is_some) || self.hands.iter().any(Option::is_some)
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

/// The radius of `wall`'s arc, if it is curved as clients draw one: any other shape, or too tight an arc, is flat.
fn arc(wall: &Wall) -> Option<f32> {
    match wall.shape {
        tomori_protocol::WallShape::Arc { radius } if radius.is_finite() && radius >= 1.0 => {
            Some(radius)
        }
        _ => None,
    }
}

/// Where each of `wall`'s slots stands, as clients draw a panel in it: a flat wall's in its plane, three columns of two
/// rows; a curved wall's along its arc, left to right, each facing the point the arc curves about.
fn slots(wall: &Wall) -> Vec<Rectangle> {
    let Some(radius) = arc(wall) else {
        return (0..WALL_SLOTS as usize)
            .map(|slot| Rectangle::at(&wall.pose, [COLUMNS[slot % 3], ROWS[slot / 3], LIFT], PANEL))
            .collect();
    };
    let pitch = (PANEL.0 * ARC_SCALE + ARC_GAP) / radius;
    (0..WALL_SLOTS)
        .map(|slot| {
            let angle = (slot as f32 - (WALL_SLOTS as f32 - 1.0) / 2.0) * pitch;
            let (sin, cos) = (angle / 2.0).sin_cos();
            // The slot's own pose: along the arc, turned about the vertical to face the arc's middle.
            let turn = [0.0, -sin, 0.0, cos];
            let [x, y, z, w] = wall.pose.orientation;
            let [_, b, _, d] = turn;
            let orientation = [x * d - z * b, y * d + w * b, z * d + x * b, w * d - y * b];
            let at = [radius * angle.sin(), 0.0, radius * (1.0 - angle.cos())];
            let pose = Pose {
                position: add(wall.pose.position, rotate(wall.pose.orientation, at)),
                orientation,
            };
            Rectangle::at(
                &pose,
                [0.0, 0.0, LIFT],
                (PANEL.0 * ARC_SCALE, PANEL.1 * ARC_SCALE),
            )
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

/// What `wall` of `projection` is called, as the agent is told and may name it: the Tomori wall, or an environment's
/// wall or an added wall by its number among those of its kind, from 1.
pub(crate) fn wall_name(projection: &Projection, wall: &Wall) -> String {
    let number = projection
        .walls
        .iter()
        .filter(|other| other.kind == wall.kind)
        .position(|other| other.id == wall.id)
        .map_or(1, |index| index + 1);
    match wall.kind {
        WallKind::Tomori => "Tomori wall".to_owned(),
        WallKind::Environment => format!("environment wall {number}"),
        _ => format!("added wall {number}"),
    }
}

/// The parts of `wall` a ray can meet: its slots, and, for a flat wall, the board they stand on.
fn surfaces(wall: &Wall) -> (Vec<Rectangle>, Option<Rectangle>) {
    let board = arc(wall)
        .is_none()
        .then(|| Rectangle::at(&wall.pose, [0.0; 3], FLAT_WALL));
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
    /// Another person, by their name.
    Person { name: String },
}

/// What the ray from `origin` in the unit `direction` meets first of `projection`'s walls and panels and `people`'s
/// heads, with how far along it, if anything.
pub(crate) fn meet(
    origin: Vec3,
    direction: Vec3,
    projection: &Projection,
    people: &[(&str, Vec3)],
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
    for (name, head) in people {
        let offset = sub(*head, origin);
        let along = dot(offset, direction);
        if along > 0.0
            && along <= RANGE
            && length(sub(offset, scale(direction, along))) <= HEAD_RADIUS
        {
            consider(
                Met::Person {
                    name: (*name).to_owned(),
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
        .map(|wall| {
            let (slots, board) = surfaces(wall);
            let distance = slots
                .iter()
                .chain(board.as_ref())
                .map(|rectangle| rectangle.distance(point))
                .fold(f32::INFINITY, f32::min);
            (wall, distance)
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
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
        Met::Person { name } => json!({ "kind": "person", "name": name }),
    };
    described["distance"] = json!(round(along, 2));
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
    people: &[(&str, Vec3)],
    now: Instant,
) -> Value {
    let role = serde_json::to_value(person.role).unwrap_or(Value::Null);
    let mut report = json!({
        "name": person.name,
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
    let others: Vec<(&str, Vec3)> = people
        .iter()
        .filter(|(name, _)| *name != person.name)
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
/// reason about where to put what it shows: each wall's identity, name, kind, where it stands and faces, its size, and
/// which of its slots hold which panels and which are free.
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
            let used: Vec<Value> = (0..WALL_SLOTS)
                .filter_map(|slot| {
                    held.iter().find(|(at, _)| *at == slot).map(|(_, panel)| {
                        json!({
                            "slot": slot,
                            "panel": panel.id,
                            "title": title(panel),
                            "code": panel.code.is_some(),
                            "pinned": panel.pinned,
                        })
                    })
                })
                .collect();
            let free: Vec<u32> = (0..WALL_SLOTS)
                .filter(|slot| !held.iter().any(|(at, _)| at == slot))
                .collect();
            // A wall's face looks along its +Z.
            let (yaw, _) = angles(rotate(wall.pose.orientation, [0.0, 0.0, 1.0]));
            let (kind, shape, size) = match (wall.kind, arc(wall)) {
                (_, Some(radius)) => {
                    let slots = slots(wall);
                    let width = length(sub(slots[0].middle, slots[slots.len() - 1].middle))
                        + PANEL.0 * ARC_SCALE;
                    (
                        wall.kind,
                        json!({ "arc": round(radius, 2) }),
                        (width, PANEL.1 * ARC_SCALE),
                    )
                }
                (kind, None) => (kind, json!("flat"), FLAT_WALL),
            };
            json!({
                "id": wall.id,
                "name": wall_name(projection, wall),
                "kind": serde_json::to_value(kind).unwrap_or(Value::Null),
                "shape": shape,
                "position": position(wall.pose.position),
                "facing_yaw": round(yaw, 0),
                "size": [round(size.0, 2), round(size.1, 2)],
                "used": used,
                "free": free,
            })
        })
        .collect::<Vec<_>>();
    let floating = projection
        .panels
        .iter()
        .filter_map(|panel| match &panel.place {
            Place::Free { pose } => Some(json!({
                "panel": panel.id,
                "title": title(panel),
                "position": position(pose.position),
                "pinned": panel.pinned,
            })),
            _ => None,
        })
        .collect::<Vec<_>>();
    (json!(walls), json!(floating))
}

/// How far in front of someone a panel the agent floats in front of them stands, in metres, and how far below their
/// eyes: within reach, and low enough to read without hiding what is behind it.
const FRONT: (f32, f32) = (1.2, 0.15);

/// Where the agent's panel goes when it asks for none, for whoever asked, whose head was at `asker` as they began
/// speaking: where they were looking, or onto the Tomori wall when nobody asked.
pub(crate) fn by_default(asker: Option<&Pose>, projection: &Projection) -> Destination {
    asker.map_or(Destination::Tomori, |head| looking_at(head, projection))
}

/// Where someone whose head is at `head` is looking, for a panel: the wall the ray from their eyes meets, in the slot
/// it meets if that is empty, or the wall of the panel it meets; else the wall in their view nearest where they look;
/// else the wall nearest them. A person in the way is looked past.
fn looking_at(head: &Pose, projection: &Projection) -> Destination {
    match meet(head.position, forward(head), projection, &[]) {
        Some((Met::Wall { wall, slot }, _)) => Destination::Wall { wall, slot },
        Some((
            Met::Panel {
                wall: Some((wall, _)),
                ..
            },
            _,
        )) => Destination::Wall { wall, slot: None },
        _ => walls_in_view(&projection.walls, head).first().map_or(
            Destination::Nearest(head.position),
            |(wall, _)| Destination::Wall {
                wall: wall.id.clone(),
                slot: None,
            },
        ),
    }
}

/// Floating in front of someone whose head is at `head`, facing them, level, the way they face about the vertical, or,
/// looking straight up or down, the way the top of their head faces.
fn in_front(head: &Pose) -> Destination {
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
    let at = [px + x * FRONT.0, py - FRONT.1, pz + z * FRONT.0];
    Destination::Free(Pose::upright_facing(at, [px, py - FRONT.1, pz]))
}

/// Where `place` puts the agent's panel, in `projection`, for whoever asked, whose head was at `asker` as they began
/// speaking, among `people`, the names and heads of the participants in the space as they last reported; or why it
/// cannot, as when it names someone who has not reported or a wall that is not here. A form that names nobody is about
/// whoever asked.
pub(crate) fn destination(
    place: &Placement,
    asker: Option<&Pose>,
    people: &[(String, Pose)],
    projection: &Projection,
) -> Result<Destination, String> {
    let whose = |name: &Option<String>| -> Result<Pose, String> {
        match name {
            Some(name) => people
                .iter()
                .find(|(person, _)| person.to_lowercase() == name.to_lowercase())
                .map(|(_, head)| *head)
                .ok_or_else(|| {
                    format!("Nobody called {name:?} in this space has reported where they are.")
                }),
            None => asker.copied().ok_or_else(|| {
                "Nobody asked for this as they spoke, so name whom it is for.".to_owned()
            }),
        }
    };
    Ok(match place {
        Placement::Near(name) => Destination::Nearest(whose(name)?.position),
        Placement::Looking(name) => looking_at(&whose(name)?, projection),
        Placement::Front(name) => in_front(&whose(name)?),
        Placement::Wall(named) => {
            let named = named.trim();
            let named = named
                .get(..4)
                .filter(|the| the.eq_ignore_ascii_case("the "))
                .map_or(named, |_| &named[4..]);
            let wall = projection
                .walls
                .iter()
                .find(|wall| {
                    wall.id.eq_ignore_ascii_case(named)
                        || wall_name(projection, wall).eq_ignore_ascii_case(named)
                })
                .ok_or_else(|| format!("There is no wall called {named:?} here."))?;
            Destination::Wall {
                wall: wall.id.clone(),
                slot: None,
            }
        }
    })
}

/// How the `presence` tool's numbers are to be read.
pub(crate) const COORDINATES: &str = "Positions are in the space's scene coordinates, in metres: y is up, and home, where everyone starts, is at the origin, facing -z, towards the Tomori wall by default. Yaw is in degrees clockwise from -z as seen from above, so 90 faces +x; pitch is in degrees above the horizontal. A wall's facing_yaw is the way its face looks. Panels hold their wall's slots, numbered from its top left as seen from in front, left to right then down.";

#[cfg(test)]
mod tests;
