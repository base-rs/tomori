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
        Met::Person { name } => json!({ "kind": "person", "name": name }),
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
/// reason about where to put what it shows: each wall's identity, name, kind, where it stands, faces and tilts, its size, and
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
/// speaking: where they were looking, or onto the Tomori wall when nobody asked.
pub(crate) fn by_default(asker: Option<&Pose>, projection: &Projection) -> Destination {
    asker.map_or(Destination::Tomori, |head| looking_at(head, projection))
}

/// How far from someone's eyes a wall may be, in metres, for the agent's panel to be put on it for them by where they
/// look: within it, a flat wall's text, at its 1.9 mm font pixel, is about as sharp as the curved Tomori wall's, at
/// seven tenths of that, from home, about 2.6 m away, which is as far as walls are designed to be read from.
/// Comfortable reading, an x-height of about 0.45° (see `design/text.md`), would need a slot's text within about 1.4 m,
/// nearer than any wall stands from home, so the cap follows the walls' own design rather than that. Beyond it, as on
/// a wall 7 m away, a slot's text is too small to read without pulling the panel off. See `design/projection.md`.
pub(crate) const READING: f32 = 4.0;

/// How far from someone's eyes `wall` may be for the agent's panel to be put on it for them by where they look:
/// [`READING`], or as many times further as its slots' text is larger than a standard slot's, since its text is as
/// legible from that far, so that a wall five times as large is read from 20 m. A smaller wall keeps [`READING`],
/// since walls stand nearer than it from home. See `design/projection.md`.
pub(crate) fn reading(wall: &Wall) -> f32 {
    READING * wall.scale().max(1.0)
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

/// Where someone whose head is at `head` is looking, for a panel, of the walls within [`READING`] of their eyes: the
/// nearest of those they look at, which are the wall the ray from their eyes meets, in the slot it meets if that is
/// empty, and those with a slot within [`GAZE`] of the ray; else the nearest wall in their view; else the nearest wall;
/// else, with no wall within reading, floating in front of them. A person in the way is looked past.
fn looking_at(head: &Pose, projection: &Projection) -> Destination {
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
    let readable: Vec<(&Wall, f32)> = projection
        .walls
        .iter()
        .map(|wall| (wall, wall_distance(wall, eyes)))
        .filter(|(wall, distance)| *distance <= reading(wall))
        .collect();
    let nearest = |walls: &mut dyn Iterator<Item = &(&Wall, f32)>| {
        walls
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(wall, _)| wall.id.clone())
    };
    // The walls looked at: the one met, and those with a slot near the ray.
    let looked = nearest(&mut readable.iter().filter(|(wall, _)| {
        met.as_ref().is_some_and(|(id, _)| *id == wall.id)
            || slots(wall)
                .iter()
                .any(|slot| angle(ahead, sub(slot.middle, eyes)) <= GAZE)
    }));
    if let Some(wall) = looked {
        let slot = met.and_then(|(id, slot)| slot.filter(|_| id == wall));
        return Destination::Wall { wall, slot };
    }
    let in_view = walls_in_view(&projection.walls, head);
    let shown = nearest(
        &mut readable
            .iter()
            .filter(|(wall, _)| in_view.iter().any(|(seen, _)| seen.id == wall.id)),
    );
    match shown {
        Some(wall) => Destination::Wall { wall, slot: None },
        None if !readable.is_empty() => Destination::Nearest(eyes),
        None => in_front(head),
    }
}

/// Floating in front of someone whose head is at `head`, facing them, level, the way they face about the vertical, or,
/// looking straight up or down, the way the top of their head faces.
fn in_front(head: &Pose) -> Destination {
    Destination::Free(ahead_of(head, FRONT))
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
    people: &[(String, Pose)],
    projection: &Projection,
) -> Result<String, String> {
    let by_name = Placement::Wall(named.to_owned(), None);
    if let Ok(Destination::Wall { wall, .. }) = destination(&by_name, asker, people, projection) {
        return Ok(wall);
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
        // Named after `wall`, it is a wall by its identity or name, which no wall here has.
        Placement::Wall(named, _) => {
            return Err(format!("There is no wall called {named:?} here."));
        }
    };
    let head = head_of(name.as_deref(), asker, people)?;
    // The wall they are looking at, if they are, or else the one nearest them; there is always the Tomori wall.
    let seen = looking.then(|| looking_at(&head, projection));
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
    people: &[(String, Pose)],
) -> Result<Pose, String> {
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
    let whose = |name: &Option<String>| head_of(name.as_deref(), asker, people);
    Ok(match place {
        Placement::Near(name) => Destination::Nearest(whose(name)?.position),
        Placement::Looking(name) => looking_at(&whose(name)?, projection),
        Placement::Front(name) => in_front(&whose(name)?),
        Placement::Wall(named, slot) => {
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
            // A slot it does not have is refused, rather than taken as its most prominent free one.
            if let Some(slot) = slot.filter(|slot| *slot >= wall.slots()) {
                return Err(format!(
                    "The {} has no slot {slot}: its slots are numbered from 0 to {}.",
                    wall_name(projection, wall),
                    wall.slots() - 1
                ));
            }
            Destination::Wall {
                wall: wall.id.clone(),
                slot: *slot,
            }
        }
    })
}

/// How far in front of someone the agent stands when it goes to them, in metres, and how far below their eyes: near
/// enough to look at something together, and far enough not to crowd them.
const BEFORE: (f32, f32) = (1.5, 0.2);

/// Where the agent stands when it goes to someone whose head is at `head`: [`BEFORE`] in front of them, level, the way
/// they face about the vertical, facing them. See `design/environments.md`.
pub(crate) fn before(head: &Pose) -> Pose {
    let ahead = forward(head);
    let (x, z) = match ahead[0].hypot(ahead[2]) {
        level if level > 1e-3 => (ahead[0] / level, ahead[2] / level),
        _ => (0.0, -1.0),
    };
    let [px, py, pz] = head.position;
    let at = [px + x * BEFORE.0, py - BEFORE.1, pz + z * BEFORE.0];
    Pose::at(at).facing_towards([px, at[1], pz])
}

/// Where the agent is, as the `presence` and `move_agent` tools say: where it stands, which way it faces, whether it is
/// at its home, and where that is. See `design/environments.md`.
pub(crate) fn agent(anchors: &tomori_protocol::Anchors, summoned: Option<Pose>) -> Value {
    let home = anchors.agent.unwrap_or_else(|| {
        Pose::at(crate::stage::AGENT).facing_towards([0.0, crate::stage::AGENT[1], 0.0])
    });
    let at = summoned.unwrap_or(home);
    // The way it faces, its +Z, clockwise from −Z.
    let facing = rotate(at.orientation, [0.0, 0.0, 1.0]);
    json!({
        "position": position(at.position),
        "facing_yaw": round(angles(facing).0, 0),
        "at_home": summoned.is_none(),
        "home": position(home.position),
    })
}

/// How the `presence` tool's numbers are to be read.
pub(crate) const COORDINATES: &str = "Positions are in the space's scene coordinates, in metres: y is up, and home, where everyone arrives, wherever the environment has them arrive, is at the origin, facing -z. The agent stands at its own home, which the environment chooses, unless someone summoned it elsewhere. Yaw is in degrees clockwise from -z as seen from above, so 90 faces +x; pitch is in degrees above the horizontal. A wall's facing_yaw is the way its face looks. Panels hold their wall's slots, numbered from its top left as seen from in front, left to right then down.";

#[cfg(test)]
mod tests;
