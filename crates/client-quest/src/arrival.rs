// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The arrival scene: servers, spaces and the creation rings as objects placed in the world, each with its label. See `design/arrival.md`.
//!
//! This is free of rendering code. [`crate::conversation::Conversation::arrival`] describes what to show as a [`View`], [`Scene::layout`] places it in scene coordinates, and its [`Scene::targets`] are what the [`crate::pointing::Pointer`] points at. How each object looks is decided only by the renderer, from its [`Kind`], so the placeholder shapes can be replaced without touching this.
use crate::{
    environment::Environment,
    pointing::{Shape, Target},
    server_choice::Source,
};
use glam::Vec3;
use std::f32::consts::{FRAC_PI_2, TAU};
use tomori_protocol::{Activity, Attention, SpaceSummary};

/// Where the participant stands when arrival begins, in scene coordinates; labels turn to face it.
pub const HOME: Vec3 = Vec3::new(0.0, 1.6, 0.0);

/// How far in front of the participant the row of servers stands, and at what height.
const ROW_DISTANCE: f32 = 1.6;
const ROW_HEIGHT: f32 = 1.35;
/// The widest the row may spread to either side, in radians, and the most room between neighbours.
const ROW_HALF_ANGLE: f32 = 1.0;
const ROW_STEP: f32 = 0.3;
/// How many objects a row holds before another row starts above it, and how far above.
const ROW_CAPACITY: usize = 12;
const ROW_RISE: f32 = 0.36;
/// The centre of the row, where a chosen server glides to and stays, at the centre of its orbit.
pub const CENTRE: Vec3 = Vec3::new(0.0, ROW_HEIGHT, -ROW_DISTANCE);
/// The connected server's orbit: its spaces and the create object stand evenly around it, in the upright plane through [`CENTRE`] facing the participant, so every object is about as far away and pointing tells them apart by direction alone.
const ORBIT_RADIUS: f32 = 0.55;
/// The least room between neighbours on an orbit, which decides how many it holds, and how far outside the last each further orbit stands.
const ORBIT_SPACING: f32 = 0.42;
const ORBIT_STEP: f32 = 0.4;
/// While its rings are open, the create object comes forward from its place at the bottom of the orbit to here, nearer and lower, so its rings open in front of the spaces and below the connected server.
pub const CREATE: Vec3 = Vec3::new(0.0, 0.85, -1.1);
/// Where the space being joined comes to, near the participant, before its environment grows out of it.
pub const APPROACH: Vec3 = Vec3::new(0.0, 1.4, -0.8);
/// A ring's smallest radius, and the arc each of its objects needs.
const RING_RADIUS: f32 = 0.3;
const RING_SPACING: f32 = 0.17;
/// The heading stands above the row, or this far above the outermost orbit.
const HEADING: Vec3 = Vec3::new(0.0, 1.85, -1.7);
const HEADING_ABOVE: f32 = 0.3;
/// Pointing is forgiving: an object's target is larger than its shape.
const TARGET_MARGIN: f32 = 1.5;
/// Labels are clipped to this many characters; the full text of the object pointed at is shown in the heading.
pub const LABEL_COLUMNS: usize = 22;
/// How far above its server a jump object stands: clear of the server and of a row above it.
const JUMP_ABOVE: f32 = 0.18;

/// What an arrival object stands for, and so what choosing it does.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    /// A server in the list, by address.
    Server(String),
    /// The object beside a server, by its address, that connects to it and joins its most urgent space in one choice.
    Jump(String),
    /// The server connected to, by address, shown as chosen; choosing it returns to the servers.
    Connected(String),
    /// A space on the server, by identifier.
    Space(String),
    /// The object that creates a space. While its rings are open, choosing it steps back one ring.
    Create,
    /// A choice on the backend ring, by name.
    Backend(String),
    /// A choice on the directory ring, by path.
    Directory(String),
    /// A choice on the session ring: an existing session to attach, or `None` for a new one.
    Session(Option<String>),
    /// A choice on the environment ring, by identifier.
    Environment(String),
    /// The object that pairs this device as an owner, on the connected server's orbit. See `design/security.md`.
    Pair,
    /// An item of the hand menu. See `design/hand-menu.md`.
    Menu(crate::hand_menu::MenuKey),
}

/// How an object should look, as far as arrival knows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Server {
        source: Source,
        /// Whether it can be joined; an incompatible server is shown but can't be chosen.
        available: bool,
        /// The most urgent attention it advertises any of its spaces as needing.
        attention: Option<Attention>,
    },
    /// The jump object above a server whose spaces need attention, showing the most urgent.
    Jump { attention: Attention },
    /// The server connected to.
    Connected,
    Space {
        activity: Activity,
        participants: usize,
        /// A hue in `0..1` derived from the space's identifier alone, so the space looks the same to everyone and on every visit.
        hue: f32,
        /// What about the space needs a participant, as the server lists it.
        attention: Option<Attention>,
    },
    /// The create object, `open` while its rings are shown.
    Create { open: bool },
    /// A choice on one of the creation rings.
    Choice,
    /// The pairing object, `pending` while its code waits to be confirmed.
    Pair { pending: bool },
    /// An item of the hand menu, or the cue on the wrist.
    Menu(crate::hand_menu::MenuKind),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub key: Key,
    pub kind: Kind,
    pub label: String,
    /// A second line, such as a space's activity or a server's address.
    pub detail: Option<String>,
    /// Chosen with the stick, which still works alongside pointing.
    pub selected: bool,
}

/// The step a creation ring offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Backend,
    Directory,
    /// Only for backends that can attach to an existing session.
    Session,
    /// Only when the server offers more than one environment.
    Environment,
}

impl Step {
    pub fn title(self) -> &'static str {
        match self {
            Step::Backend => "Choose a backend",
            Step::Directory => "Choose a folder",
            Step::Session => "New or existing session",
            Step::Environment => "Choose an environment",
        }
    }
}

/// The choices of one creation ring, around the create object.
#[derive(Clone, Debug, PartialEq)]
pub struct Ring {
    pub step: Step,
    pub items: Vec<Item>,
}

/// What arrival shows, before it is placed in the world.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct View {
    pub heading: String,
    /// Progress, a hint or an error.
    pub status: Option<String>,
    /// The server connected to, once one is chosen.
    pub connected: Option<Item>,
    /// The servers, or the spaces followed by the create object.
    pub items: Vec<Item>,
    /// The creation ring that is open, if any.
    pub ring: Option<Ring>,
    /// The space being joined, which comes towards the participant while everything else recedes.
    pub joining: Option<Key>,
}

/// How a label's lines line up with its anchor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    /// The label extends to the right of its anchor.
    Left,
    Centre,
    /// The label extends to the left of its anchor.
    Right,
}

/// Text anchored to a point in the world, which turns to face the viewer, or [`HOME`] when the viewer is not known.
#[derive(Clone, Debug, PartialEq)]
pub struct Label {
    /// The top of the label, at the side given by `align`.
    pub anchor: Vec3,
    pub lines: Vec<String>,
    pub align: Align,
    /// The size of one font pixel, in metres.
    pub pixel: f32,
    /// How opaque it is, from 0 to 1, as it fades in or out with its object.
    pub alpha: f32,
    /// Whether it tilts to face the viewer fully, as text held close does, such as the hand menu's, rather than only turning upright.
    pub tilt: bool,
}

/// An arrival object, placed in the world.
#[derive(Clone, Debug, PartialEq)]
pub struct Object {
    pub key: Key,
    pub kind: Kind,
    pub position: Vec3,
    /// Its half-size, in metres.
    pub size: f32,
    pub label: Label,
    /// The label and detail in full, shown in the heading while the object is pointed at.
    pub description: String,
    pub selected: bool,
    /// How faintly it is shown, from 0 to 1: 1 behind an open ring.
    pub dim: f32,
    /// How far it has appeared, from 0 to 1. Its size and label are already scaled by this.
    pub presence: f32,
    /// Whether it can be pointed at; an object on its way out can't.
    pub choosable: bool,
}

/// Which step of arrival a scene shows, which decides how a change to it is animated.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Stage {
    #[default]
    Servers,
    /// The spaces and the create object, with any creation ring.
    Lobby,
    /// A space is being joined.
    Joining(Key),
    /// Inside a space, which shows no arrival objects; `space` is `None` when it was not among them.
    Entered {
        space: Option<Key>,
        environment: Environment,
    },
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Scene {
    pub objects: Vec<Object>,
    pub heading: Option<Label>,
    pub stage: Stage,
    /// Where the viewer's head is, which labels and forms face, or `None` for [`HOME`].
    pub viewer: Option<Vec3>,
}

impl Scene {
    /// Place `view` in the world: the servers on a row in front of [`HOME`], or the connected server at the [`CENTRE`] with its spaces and the create object on its orbit, and the create object's open ring around it.
    pub fn layout(view: &View) -> Self {
        let mut objects = Vec::new();
        let stage = match &view.joining {
            Some(key) if view.items.iter().any(|item| &item.key == key) => {
                Stage::Joining(key.clone())
            }
            _ if view.connected.is_some() => Stage::Lobby,
            _ => Stage::Servers,
        };
        let heading = if let Some(item) = &view.connected {
            objects.push(row_object(item, CENTRE, false));
            let ring_open = view.ring.is_some();
            // The create object first, so that it keeps its place at the bottom however many spaces there are, then the spaces in their order, clockwise.
            let (create, spaces): (Vec<_>, Vec<_>) =
                view.items.iter().partition(|item| item.key == Key::Create);
            let orbiting: Vec<_> = create.iter().chain(&spaces).collect();
            let places = orbit(orbiting.len());
            for (item, &position) in orbiting.iter().zip(&places) {
                let creating = ring_open && item.key == Key::Create;
                let position = if creating { CREATE } else { position };
                objects.push(row_object(item, position, ring_open && !creating));
            }
            let top = places.iter().map(|place| place.y).fold(CENTRE.y, f32::max);
            HEADING.with_y(top + HEADING_ABOVE)
        } else {
            // Jump objects stand above their servers rather than in the row.
            let servers: Vec<&Item> = view
                .items
                .iter()
                .filter(|item| !matches!(item.key, Key::Jump(_)))
                .collect();
            let jumps = view.items.iter().filter_map(|item| match &item.key {
                Key::Jump(address) => Some((item, address)),
                _ => None,
            });
            // More than a row holds continue on rows above.
            let rows = servers.len().div_ceil(ROW_CAPACITY).max(1);
            for (number, row) in servers.chunks(ROW_CAPACITY).enumerate() {
                let step = if row.len() > 1 {
                    ROW_STEP.min(2.0 * ROW_HALF_ANGLE / (row.len() - 1) as f32)
                } else {
                    0.0
                };
                let start = -step * (row.len() - 1) as f32 / 2.0;
                let height = ROW_HEIGHT + number as f32 * ROW_RISE;
                for (index, item) in row.iter().enumerate() {
                    let position = on_arc(start + step * index as f32, ROW_DISTANCE, height);
                    objects.push(row_object(item, position, false));
                }
            }
            for (jump, address) in jumps {
                let server = objects
                    .iter()
                    .find(|object| matches!(&object.key, Key::Server(found) if found == address))
                    .map(|object| object.position);
                if let Some(server) = server {
                    objects.push(jump_object(jump, server + Vec3::Y * JUMP_ABOVE));
                }
            }
            HEADING + Vec3::Y * (rows - 1) as f32 * ROW_RISE
        };
        fit_labels(&mut objects);
        if let Some(ring) = &view.ring
            && view.items.iter().any(|item| item.key == Key::Create)
        {
            objects.extend(ring_objects(ring, CREATE));
        }
        // While a space is joined, only it remains, near the participant; the rest recede.
        if let Stage::Joining(key) = &stage
            && let Some(item) = view.items.iter().find(|item| &item.key == key)
        {
            objects = vec![row_object(item, APPROACH, false)];
        }
        let mut lines = vec![clip(&view.heading, 40)];
        lines.extend(view.status.iter().map(|status| clip(status, 40)));
        Self {
            objects,
            heading: Some(Label {
                anchor: heading,
                lines,
                align: Align::Centre,
                pixel: 0.0032,
                alpha: 1.0,
                tilt: false,
            }),
            stage,
            viewer: None,
        }
    }

    /// Inside a space: no objects, only what is needed to grow its environment out of its object.
    pub fn entered(space: Option<Key>, environment: Environment) -> Self {
        Self {
            stage: Stage::Entered { space, environment },
            ..Default::default()
        }
    }

    /// What can be pointed at: every object that is choosable and at least half appeared, a little larger than it looks. The create object stands straight ahead, so a box fits its plus.
    pub fn targets(&self) -> Vec<Target<Key>> {
        self.objects
            .iter()
            .filter(|object| object.choosable && object.presence >= 0.5)
            .map(|object| {
                let reach = object.size * TARGET_MARGIN;
                Target {
                    key: object.key.clone(),
                    shape: match object.kind {
                        Kind::Create { .. } => Shape::Box {
                            center: object.position,
                            half_extents: Vec3::new(reach, reach, object.size * 0.6),
                        },
                        // A row of the hand menu is chosen by its marker or its label, which runs to the right of it, and is no taller than a row, so that rows never overlap.
                        Kind::Menu(_) => {
                            let right = (object.label.anchor - object.position)
                                .with_y(0.0)
                                .normalize_or_zero();
                            let length = crate::hand_menu::ROW_LENGTH;
                            Shape::Box {
                                center: object.position + right * (length / 2.0 - object.size),
                                half_extents: Vec3::new(
                                    right.x.abs() * length / 2.0 + 0.01,
                                    crate::hand_menu::ROW * 0.45,
                                    right.z.abs() * length / 2.0 + 0.01,
                                ),
                            }
                        }
                        _ => Shape::Sphere {
                            center: object.position,
                            radius: reach,
                        },
                    },
                }
            })
            .collect()
    }

    pub fn object(&self, key: &Key) -> Option<&Object> {
        self.objects.iter().find(|object| &object.key == key)
    }
}

/// A point `distance` from [`HOME`] at `height`, `angle` radians to the right of straight ahead.
fn on_arc(angle: f32, distance: f32, height: f32) -> Vec3 {
    Vec3::new(angle.sin() * distance, height, -angle.cos() * distance)
}

/// The places of `count` objects on the connected server's orbits, the first at the bottom and the rest clockwise as seen from [`HOME`]. Each orbit holds as many as fit with [`ORBIT_SPACING`] between them before another starts outside it, half a step round, so its objects stand between those of the orbit inside.
fn orbit(count: usize) -> Vec<Vec3> {
    let mut places = Vec::with_capacity(count);
    let mut radius = ORBIT_RADIUS;
    let mut turn = 0.0;
    while places.len() < count {
        let capacity = (TAU * radius / ORBIT_SPACING) as usize;
        let here = capacity.min(count - places.len());
        for index in 0..here {
            let angle = -FRAC_PI_2 - (index as f32 + turn) * TAU / here as f32;
            let (sin, cos) = angle.sin_cos();
            places.push(CENTRE + Vec3::new(cos, sin, 0.0) * radius);
        }
        radius += ORBIT_STEP;
        turn = 0.5;
    }
    places
}

/// How far a label's background reaches to either side of its anchor, when it is centred.
fn half_width(label: &Label) -> f32 {
    let columns = label.lines.iter().map(|line| line.len()).max().unwrap_or(0);
    (columns as f32 * 9.0 + 6.0) * label.pixel / 2.0
}

/// The heights a label's background spans, from its bottom to its top.
fn label_span(label: &Label) -> (f32, f32) {
    let height = (label.lines.len() as f32 * 11.0 + 4.0) * label.pixel;
    (label.anchor.y - height, label.anchor.y + 3.0 * label.pixel)
}

/// Clip labels centred beneath their objects so that none reaches more than halfway to an object beside it whose form or label shares its height. Neighbours on an orbit, or on a crowded row, then never overlap; the full text is still shown in the heading while the object is pointed at.
fn fit_labels(objects: &mut [Object]) {
    let limits: Vec<f32> = objects
        .iter()
        .map(|object| {
            let (bottom, top) = label_span(&object.label);
            objects
                .iter()
                .filter(|other| other.key != object.key)
                .filter(|other| {
                    let (label_bottom, _) = label_span(&other.label);
                    let reach = other.size * TARGET_MARGIN;
                    label_bottom.min(other.position.y - reach) < top
                        && other.position.y + reach > bottom
                })
                .map(|other| (other.position - object.position).with_y(0.0).length() / 2.0)
                .fold(f32::INFINITY, f32::min)
        })
        .collect();
    for (object, limit) in objects.iter_mut().zip(limits) {
        let label = &mut object.label;
        if half_width(label) <= limit {
            continue;
        }
        let columns = ((limit * 2.0 / label.pixel - 6.0) / 9.0).floor().max(6.0) as usize;
        for line in &mut label.lines {
            *line = clip(line, columns);
        }
    }
}

fn size(kind: Kind) -> f32 {
    match kind {
        Kind::Server { .. } => 0.07,
        Kind::Jump { .. } => 0.04,
        Kind::Connected => 0.07,
        Kind::Space { .. } => 0.08,
        Kind::Create { .. } => 0.08,
        Kind::Choice => 0.05,
        Kind::Pair { .. } => 0.06,
        Kind::Menu(_) => 0.009,
    }
}

fn description(item: &Item) -> String {
    match &item.detail {
        Some(detail) => format!("{}: {detail}", item.label),
        None => item.label.clone(),
    }
}

fn lines(item: &Item) -> Vec<String> {
    let mut lines = vec![clip(&item.label, LABEL_COLUMNS)];
    lines.extend(item.detail.iter().map(|detail| clip(detail, LABEL_COLUMNS)));
    lines
}

/// An object with its label centred beneath it.
fn row_object(item: &Item, position: Vec3, dimmed: bool) -> Object {
    let size = size(item.kind);
    Object {
        key: item.key.clone(),
        kind: item.kind,
        position,
        size,
        label: Label {
            anchor: position - Vec3::Y * (size * 1.6 + 0.03),
            lines: lines(item),
            align: Align::Centre,
            pixel: 0.0026,
            alpha: 1.0,
            tilt: false,
        },
        description: description(item),
        selected: item.selected,
        dim: if dimmed { 1.0 } else { 0.0 },
        presence: 1.0,
        choosable: true,
    }
}

/// A jump object at `position`, with its short label above it, clear of its server beneath.
fn jump_object(item: &Item, position: Vec3) -> Object {
    let size = size(item.kind);
    Object {
        key: item.key.clone(),
        kind: item.kind,
        position,
        size,
        label: Label {
            anchor: position + Vec3::Y * (size * 1.4 + 0.035),
            lines: vec![clip(&item.label, LABEL_COLUMNS)],
            align: Align::Centre,
            pixel: 0.0022,
            alpha: 1.0,
            tilt: false,
        },
        description: description(item),
        selected: item.selected,
        dim: 0.0,
        presence: 1.0,
        choosable: true,
    }
}

/// A ring's objects around `centre`, starting at the top and going clockwise as seen from [`HOME`], each labelled on its outer side.
fn ring_objects(ring: &Ring, centre: Vec3) -> Vec<Object> {
    let count = ring.items.len();
    let radius = RING_RADIUS.max(count as f32 * RING_SPACING / TAU);
    let facing = facing(centre);
    let right = Vec3::Y.cross(facing);
    ring.items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let angle = FRAC_PI_2 - index as f32 * TAU / count as f32;
            let (sin, cos) = angle.sin_cos();
            let position = centre + (right * cos + Vec3::Y * sin) * radius;
            let size = size(item.kind);
            let gap = size * 1.4;
            let (align, anchor) = if cos > 0.3 {
                (Align::Left, position + right * gap + Vec3::Y * 0.012)
            } else if cos < -0.3 {
                (Align::Right, position - right * gap + Vec3::Y * 0.012)
            } else if sin > 0.0 {
                (Align::Centre, position + Vec3::Y * (gap + 0.05))
            } else {
                (Align::Centre, position - Vec3::Y * gap)
            };
            Object {
                key: item.key.clone(),
                kind: item.kind,
                position,
                size,
                label: Label {
                    anchor,
                    lines: lines(item),
                    align,
                    pixel: 0.0022,
                    alpha: 1.0,
                    tilt: false,
                },
                description: description(item),
                selected: item.selected,
                dim: 0.0,
                presence: 1.0,
                choosable: true,
            }
        })
        .collect()
}

/// The horizontal direction from `point` towards [`HOME`], which labels face.
pub fn facing(point: Vec3) -> Vec3 {
    facing_from(point, None, false)
}

/// The direction from `point` towards `viewer`, or [`HOME`]: horizontal unless `tilt`.
pub fn facing_from(point: Vec3, viewer: Option<Vec3>, tilt: bool) -> Vec3 {
    let towards = viewer.unwrap_or(HOME) - point;
    let towards = if tilt { towards } else { towards.with_y(0.0) };
    towards.try_normalize().unwrap_or(Vec3::Z)
}

/// How urgent `attention` is, most urgent first: input, then a problem, then a reply, then a level this build does not know. `None` needs nothing.
pub fn urgency(attention: Option<Attention>) -> Option<u8> {
    attention.map(|attention| match attention {
        Attention::Input => 0,
        Attention::Problem => 1,
        Attention::Reply => 2,
        Attention::Unknown => 3,
    })
}

/// The space that most needs attention, by its index in `spaces`: the most urgent level, then the one that has needed it longest, then the first listed. `None` when none needs any. Spaces keep their places; this only chooses among them. See `design/arrival.md`.
pub fn most_urgent(spaces: &[SpaceSummary]) -> Option<usize> {
    spaces
        .iter()
        .enumerate()
        .filter_map(|(index, space)| {
            let urgency = urgency(space.attention)?;
            Some((
                urgency,
                std::cmp::Reverse(space.attention_age.unwrap_or(0)),
                index,
            ))
        })
        .min()
        .map(|(_, _, index)| index)
}

/// A few words for what a space needs, for its label.
pub fn attention_label(attention: Attention) -> &'static str {
    match attention {
        Attention::Input => "needs you",
        Attention::Problem => "problem",
        Attention::Reply => "new reply",
        Attention::Unknown => "needs attention",
    }
}

/// The colour that marks `attention`, the same on servers, jump objects and spaces: amber for input, red for a problem, a soft white for a reply.
pub fn attention_colour(attention: Attention) -> [f32; 3] {
    match attention {
        Attention::Input => [1.0, 0.7, 0.15],
        Attention::Problem => [1.0, 0.25, 0.2],
        Attention::Reply | Attention::Unknown => [0.9, 0.93, 1.0],
    }
}

/// A hue in `0..1` from `id` alone, using FNV-1a so it is the same on every build and device.
pub fn hue(id: &str) -> f32 {
    let mut hash: u32 = 0x811c_9dc5;
    for byte in id.bytes() {
        hash ^= u32::from(byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    (hash >> 8) as f32 / (1 << 24) as f32
}

/// `text` as ASCII, on one line of at most `columns` characters.
pub fn clip(text: &str, columns: usize) -> String {
    let ascii = crate::caption::wrapped_lines(&text.replace('\n', " ")).join(" ");
    if ascii.chars().count() <= columns {
        ascii
    } else {
        format!("{}...", &ascii[..columns.saturating_sub(3)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_of_object_has_a_size_and_every_need_a_label() {
        assert_eq!(size(Kind::Pair { pending: false }), 0.06);
        assert_eq!(size(Kind::Menu(crate::hand_menu::MenuKind::Button)), 0.009);
        assert_eq!(attention_label(Attention::Unknown), "needs attention");
    }
    use crate::pointing::{Pointer, Ray};

    fn item(key: Key, kind: Kind, label: &str) -> Item {
        Item {
            key,
            kind,
            label: label.into(),
            detail: None,
            selected: false,
        }
    }

    fn space(id: &str) -> Item {
        item(
            Key::Space(id.into()),
            Kind::Space {
                activity: Activity::Idle,
                participants: 0,
                hue: hue(id),
                attention: None,
            },
            id,
        )
    }

    fn create(open: bool) -> Item {
        item(Key::Create, Kind::Create { open }, "New space")
    }

    fn lobby(spaces: usize, ring: Option<Ring>) -> View {
        let mut items: Vec<_> = (0..spaces).map(|n| space(&format!("space-{n}"))).collect();
        items.push(create(ring.is_some()));
        View {
            heading: "Studio".into(),
            status: None,
            connected: Some(item(
                Key::Connected("studio:8765".into()),
                Kind::Connected,
                "Studio",
            )),
            items,
            ring,
            joining: None,
        }
    }

    fn ring(count: usize) -> Ring {
        Ring {
            step: Step::Backend,
            items: (0..count)
                .map(|n| {
                    item(
                        Key::Backend(format!("b{n}")),
                        Kind::Choice,
                        &format!("b{n}"),
                    )
                })
                .collect(),
        }
    }

    /// Point from [`HOME`] at `key`'s object and pull the trigger.
    fn choose(scene: &Scene, key: &Key) -> Option<Key> {
        let target = scene.object(key).unwrap().position;
        let ray = Ray::new(HOME, target - HOME).unwrap();
        let mut pointer = Pointer::default();
        pointer
            .update(Some(ray), Some(1.0), &scene.targets())
            .chosen
    }

    fn assert_separate(scene: &Scene) {
        for (index, a) in scene.objects.iter().enumerate() {
            for b in &scene.objects[index + 1..] {
                assert!(
                    a.position.distance(b.position) > (a.size + b.size) * TARGET_MARGIN,
                    "{:?} and {:?} overlap",
                    a.key,
                    b.key
                );
            }
        }
    }

    #[test]
    fn every_object_can_be_pointed_at_from_home() {
        for view in [
            lobby(0, None),
            lobby(1, None),
            lobby(6, None),
            lobby(20, None),
            lobby(3, Some(ring(2))),
            lobby(3, Some(ring(9))),
            lobby(3, Some(ring(40))),
        ] {
            let scene = Scene::layout(&view);
            assert_separate(&scene);
            for object in &scene.objects {
                assert_eq!(
                    choose(&scene, &object.key).as_ref(),
                    Some(&object.key),
                    "{:?}",
                    object.key
                );
                assert!(object.position.is_finite());
                // In front of the participant.
                assert!(object.position.z < -0.4, "{:?}", object.key);
            }
        }
    }

    #[test]
    fn the_create_object_is_shown_directly_when_there_are_no_spaces() {
        let scene = Scene::layout(&lobby(0, None));
        let keys: Vec<_> = scene.objects.iter().map(|object| &object.key).collect();
        assert_eq!(keys, [&Key::Connected("studio:8765".into()), &Key::Create]);
        let create = scene.object(&Key::Create).unwrap();
        assert!(create.position.distance(CENTRE - Vec3::Y * ORBIT_RADIUS) < 1e-5);
    }

    /// The angle of `position` around the [`CENTRE`] as seen from [`HOME`], anticlockwise from the right.
    fn around(position: Vec3) -> f32 {
        let offset = position - CENTRE;
        offset.y.atan2(offset.x)
    }

    /// How far clockwise `b` is from `a`, from 0 to a full turn.
    fn clockwise(a: f32, b: f32) -> f32 {
        (a - b).rem_euclid(TAU)
    }

    #[test]
    fn the_connected_server_is_at_the_centre_with_its_spaces_evenly_around_it() {
        for count in 1..=7 {
            let scene = Scene::layout(&lobby(count, None));
            let connected = scene.object(&Key::Connected("studio:8765".into())).unwrap();
            assert_eq!(connected.position, CENTRE);
            // The create object at the bottom, then the spaces clockwise in their order.
            let mut order = vec![Key::Create];
            order.extend((0..count).map(|n| Key::Space(format!("space-{n}"))));
            let positions: Vec<_> = order
                .iter()
                .map(|key| scene.object(key).unwrap().position)
                .collect();
            assert!((around(positions[0]) + FRAC_PI_2).abs() < 1e-5);
            let step = TAU / positions.len() as f32;
            for (index, position) in positions.iter().enumerate() {
                assert!((position.distance(CENTRE) - ORBIT_RADIUS).abs() < 1e-5);
                assert!(
                    (position.z - CENTRE.z).abs() < 1e-5,
                    "facing the participant"
                );
                // With the create object alone, the next is itself, a whole turn on.
                let next = positions[(index + 1) % positions.len()];
                let turn = clockwise(around(*position), around(next));
                let turn = if turn == 0.0 { TAU } else { turn };
                assert!((turn - step).abs() < 1e-4, "{count}: {turn} not {step}");
            }
        }
        // A space added at the end keeps the create object where it was.
        let before = Scene::layout(&lobby(3, None));
        let after = Scene::layout(&lobby(4, None));
        assert!(
            before
                .object(&Key::Create)
                .unwrap()
                .position
                .distance(after.object(&Key::Create).unwrap().position)
                < 1e-5
        );
    }

    #[test]
    fn many_spaces_continue_on_wider_orbits() {
        let scene = Scene::layout(&lobby(20, None));
        let radii: Vec<_> = scene
            .objects
            .iter()
            .filter(|object| object.position != CENTRE)
            .map(|object| object.position.distance(CENTRE))
            .collect();
        let inner = radii
            .iter()
            .filter(|radius| (**radius - ORBIT_RADIUS).abs() < 1e-4)
            .count();
        let outer = radii
            .iter()
            .filter(|radius| (**radius - ORBIT_RADIUS - ORBIT_STEP).abs() < 1e-4)
            .count();
        assert_eq!((inner, outer), (8, 13));
        // The inner orbit is full first, so the spaces there don't move as more are added.
        let fewer = Scene::layout(&lobby(7, None));
        for n in 0..7 {
            let key = Key::Space(format!("space-{n}"));
            assert!(
                fewer
                    .object(&key)
                    .unwrap()
                    .position
                    .distance(scene.object(&key).unwrap().position)
                    < 1e-5
            );
        }
        for count in [8, 21, 40, 80] {
            let scene = Scene::layout(&lobby(count, None));
            assert_separate(&scene);
            for object in &scene.objects {
                assert_eq!(choose(&scene, &object.key).as_ref(), Some(&object.key));
            }
            // The heading stays above them all.
            let top = scene
                .objects
                .iter()
                .map(|o| o.position.y)
                .fold(0.0, f32::max);
            assert!(scene.heading.unwrap().anchor.y > top + 0.2);
        }
    }

    #[test]
    fn choosing_the_connected_server_at_the_centre_returns_to_the_servers() {
        let scene = Scene::layout(&lobby(6, None));
        let ray = Ray::new(HOME, CENTRE - HOME).unwrap();
        assert_eq!(
            Pointer::default()
                .update(Some(ray), Some(1.0), &scene.targets())
                .chosen,
            Some(Key::Connected("studio:8765".into()))
        );
    }

    #[test]
    fn an_open_ring_stays_clear_of_the_connected_server() {
        for count in [1, 2, 4, 6, 9] {
            let scene = Scene::layout(&lobby(5, Some(ring(count))));
            assert_separate(&scene);
            let connected = scene.object(&Key::Connected("studio:8765".into())).unwrap();
            let towards = |object: &Object| (object.position - HOME).normalize();
            // The angle the connected server's target covers, as seen by the participant.
            let reach = |object: &Object| {
                (object.size * TARGET_MARGIN / object.position.distance(HOME)).asin()
            };
            for choice in scene.objects.iter().filter(|o| o.kind == Kind::Choice) {
                let apart = towards(choice).angle_between(towards(connected));
                assert!(apart > reach(choice) + reach(connected), "{count}");
                // Its label stays below the connected server's, as seen by the participant.
                let below = |label: &Label, y: f32| {
                    let distance = (label.anchor - HOME).with_y(0.0).length();
                    (HOME.y - y).atan2(distance)
                };
                let (bottom, _) = label_span(&connected.label);
                let (_, top) = label_span(&choice.label);
                assert!(below(&choice.label, top) > below(&connected.label, bottom));
            }
            assert_eq!(connected.dim, 0.0);
        }
    }

    #[test]
    fn labels_on_the_orbit_never_overlap() {
        for count in 0..=30 {
            let mut view = lobby(count, None);
            for item in &mut view.items {
                if matches!(item.key, Key::Space(_)) {
                    item.label = format!("A long space title {}", item.label);
                    item.detail = Some("waiting for you".into());
                }
            }
            let scene = Scene::layout(&view);
            let bounds = |label: &Label| {
                let half = half_width(label);
                let (bottom, top) = label_span(label);
                (label.anchor.x - half, label.anchor.x + half, bottom, top)
            };
            for (index, a) in scene.objects.iter().enumerate() {
                let (left, right, bottom, top) = bounds(&a.label);
                for b in &scene.objects[index + 1..] {
                    let (b_left, b_right, b_bottom, b_top) = bounds(&b.label);
                    assert!(
                        right <= b_left || b_right <= left || top <= b_bottom || b_top <= bottom,
                        "{count}: the labels of {:?} and {:?} overlap",
                        a.key,
                        b.key
                    );
                }
            }
        }
    }

    #[test]
    fn a_ring_opens_around_the_create_object_and_dims_the_spaces() {
        let scene = Scene::layout(&lobby(2, Some(ring(4))));
        let centre = scene.object(&Key::Create).unwrap().position;
        let choices: Vec<_> = scene
            .objects
            .iter()
            .filter(|object| object.kind == Kind::Choice)
            .collect();
        assert_eq!(choices.len(), 4);
        for choice in &choices {
            assert!((choice.position.distance(centre) - RING_RADIUS).abs() < 1e-4);
        }
        // The first is at the top, and they go clockwise: the second is to the right.
        assert!(choices[0].position.y > centre.y);
        assert!(choices[1].position.x > centre.x);
        assert_eq!(choices[1].label.align, Align::Left);
        assert_eq!(choices[3].label.align, Align::Right);
        assert!(
            scene
                .objects
                .iter()
                .filter(|object| matches!(object.key, Key::Space(_)))
                .all(|object| object.dim == 1.0)
        );
        assert_eq!(scene.object(&Key::Create).unwrap().dim, 0.0);
    }

    #[test]
    fn a_space_being_joined_comes_close_and_the_rest_leave() {
        let view = View {
            joining: Some(Key::Space("space-1".into())),
            ..lobby(3, None)
        };
        let scene = Scene::layout(&view);
        assert_eq!(scene.stage, Stage::Joining(Key::Space("space-1".into())));
        assert_eq!(scene.objects.len(), 1);
        assert_eq!(scene.objects[0].position, APPROACH);
        // A join for a space not shown leaves the lobby as it is.
        let unknown = View {
            joining: Some(Key::Space("elsewhere".into())),
            ..lobby(3, None)
        };
        assert_eq!(Scene::layout(&unknown).stage, Stage::Lobby);
        assert_eq!(Scene::layout(&unknown).objects.len(), 5);
    }

    #[test]
    fn only_choosable_objects_that_have_mostly_appeared_are_targets() {
        let mut scene = Scene::layout(&lobby(2, None));
        let count = scene.targets().len();
        scene.objects[0].choosable = false;
        scene.objects[1].presence = 0.4;
        assert_eq!(scene.targets().len(), count - 2);
    }

    #[test]
    fn servers_form_a_row_centred_ahead_and_bounded_to_the_sides() {
        let servers = |count: usize| View {
            heading: "Servers".into(),
            items: (0..count)
                .map(|n| {
                    item(
                        Key::Server(format!("10.0.0.{n}:8765")),
                        Kind::Server {
                            source: Source::Discovered,
                            available: true,
                            attention: None,
                        },
                        "Studio",
                    )
                })
                .collect(),
            ..Default::default()
        };
        let one = Scene::layout(&servers(1));
        assert!(one.objects[0].position.x.abs() < 1e-5);
        let many = Scene::layout(&servers(30));
        assert_separate(&many);
        let widest = many
            .objects
            .iter()
            .map(|object| object.position.x.abs() / ROW_DISTANCE)
            .fold(0.0, f32::max);
        assert!(widest <= ROW_HALF_ANGLE.sin() + 1e-4);
    }

    #[test]
    fn labels_sit_with_their_objects_and_face_home() {
        let scene = Scene::layout(&lobby(5, Some(ring(6))));
        for object in &scene.objects {
            assert!(object.label.anchor.distance(object.position) < 0.25);
            assert!(
                object
                    .label
                    .lines
                    .iter()
                    .all(|line| line.len() <= LABEL_COLUMNS)
            );
            let facing = facing(object.label.anchor);
            let towards = (HOME - object.label.anchor).with_y(0.0).normalize();
            assert!(facing.distance(towards) < 1e-5);
        }
        assert_eq!(facing(HOME), Vec3::Z);
        // Towards the viewer when known: level unless tilted.
        let below = Vec3::new(0.0, 1.0, -0.5);
        let viewer = Vec3::new(0.0, 1.5, 0.0);
        assert_eq!(facing_from(below, Some(viewer), false), Vec3::Z);
        let tilted = facing_from(below, Some(viewer), true);
        assert!(tilted.y > 0.6 && (tilted - (viewer - below).normalize()).length() < 1e-5);
        let heading = scene.heading.unwrap();
        assert_eq!(heading.lines, ["Studio"]);
    }

    #[test]
    fn a_space_hue_depends_on_its_identifier_alone() {
        assert_eq!(hue("space-1"), hue("space-1"));
        assert_ne!(hue("space-1"), hue("space-2"));
        for id in ["", "a", "space-1", "🦀"] {
            assert!((0.0..1.0).contains(&hue(id)));
        }
    }

    #[test]
    fn long_labels_are_clipped_to_ascii() {
        assert_eq!(clip("short", 10), "short");
        assert_eq!(clip("a much longer label", 10), "a much ...");
        assert_eq!(clip("two\nlines", 20), "two lines");
        assert!(clip(&"🦀".repeat(40), 10).len() <= 10);
    }

    fn summary(id: &str, attention: Option<Attention>, age: Option<u32>) -> SpaceSummary {
        SpaceSummary {
            id: id.into(),
            title: id.into(),
            backend: "fake".into(),
            directory: "/tmp".into(),
            environment: "sky".into(),
            activity: Activity::Idle,
            participants: 0,
            attention,
            attention_age: age,
        }
    }

    #[test]
    fn the_most_urgent_space_is_the_highest_level_then_the_longest_waiting() {
        assert_eq!(most_urgent(&[]), None);
        assert_eq!(most_urgent(&[summary("a", None, None)]), None);
        let spaces = [
            summary("idle", None, None),
            summary("reply", Some(Attention::Reply), Some(900)),
            summary("problem", Some(Attention::Problem), Some(5)),
            summary("new input", Some(Attention::Input), Some(10)),
            summary("old input", Some(Attention::Input), Some(60)),
            summary("unknown", Some(Attention::Unknown), Some(1000)),
        ];
        assert_eq!(most_urgent(&spaces), Some(4));
        assert_eq!(most_urgent(&spaces[..4]), Some(3));
        assert_eq!(most_urgent(&spaces[..3]), Some(2));
        assert_eq!(most_urgent(&spaces[..2]), Some(1));
        // A level this build does not know still counts, after the ones it knows.
        assert_eq!(
            most_urgent(&[spaces[0].clone(), spaces[5].clone()]),
            Some(1)
        );
        // Equal and unknown ages keep the listing's order.
        let tied = [
            summary("first", Some(Attention::Reply), None),
            summary("second", Some(Attention::Reply), Some(0)),
        ];
        assert_eq!(most_urgent(&tied), Some(0));
    }

    fn servers_needing(attention: &[Option<Attention>]) -> View {
        let mut items = Vec::new();
        for (n, attention) in attention.iter().enumerate() {
            let address = format!("10.0.0.{n}:8765");
            items.push(item(
                Key::Server(address.clone()),
                Kind::Server {
                    source: Source::Discovered,
                    available: true,
                    attention: *attention,
                },
                "Studio",
            ));
            if let Some(attention) = attention {
                items.push(item(
                    Key::Jump(address),
                    Kind::Jump {
                        attention: *attention,
                    },
                    "Jump",
                ));
            }
        }
        View {
            heading: "Servers".into(),
            items,
            ..Default::default()
        }
    }

    #[test]
    fn a_jump_stands_above_its_server_and_both_can_be_pointed_at() {
        let view = servers_needing(&[None, Some(Attention::Input), None]);
        let scene = Scene::layout(&view);
        // The servers keep the places they would have without it.
        let plain = Scene::layout(&servers_needing(&[None, None, None]));
        for n in 0..3 {
            let key = Key::Server(format!("10.0.0.{n}:8765"));
            assert_eq!(
                scene.object(&key).unwrap().position,
                plain.object(&key).unwrap().position
            );
        }
        let server = scene.object(&Key::Server("10.0.0.1:8765".into())).unwrap();
        let jump = scene.object(&Key::Jump("10.0.0.1:8765".into())).unwrap();
        assert!((jump.position - server.position - Vec3::Y * JUMP_ABOVE).length() < 1e-5);
        assert_eq!(jump.label.lines, ["Jump"]);
        assert!(jump.label.anchor.y > jump.position.y);
        for object in &scene.objects {
            assert_eq!(choose(&scene, &object.key).as_ref(), Some(&object.key));
        }
        assert_separate(&scene);

        // Crowded, and on several rows, they stay apart.
        for count in [12, 13, 30] {
            let scene = Scene::layout(&servers_needing(&vec![Some(Attention::Reply); count]));
            assert_eq!(scene.objects.len(), count * 2);
            assert_separate(&scene);
        }
        // A jump whose server is not listed is not shown.
        let mut orphan = servers_needing(&[None]);
        orphan.items.push(item(
            Key::Jump("elsewhere:1".into()),
            Kind::Jump {
                attention: Attention::Input,
            },
            "Jump",
        ));
        assert_eq!(Scene::layout(&orphan).objects.len(), 1);
    }
}
