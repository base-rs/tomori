// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The arrival scene: servers, spaces and the creation rings as objects placed in the world, each with its label. See `design/arrival.md`.
//!
//! This is free of rendering code. [`crate::conversation::Conversation::arrival`] describes what to show as a [`View`], [`Scene::layout`] places it in scene coordinates, and its [`Scene::targets`] are what the [`crate::pointing::Pointer`] points at. How each object looks is decided only by the renderer, from its [`Kind`], so the placeholder shapes can be replaced without touching this.
use crate::{
    pointing::{Shape, Target},
    server_choice::Source,
};
use glam::Vec3;
use protocol::Activity;

/// Where the participant stands when arrival begins, in scene coordinates; labels turn to face it.
pub const HOME: Vec3 = Vec3::new(0.0, 1.6, 0.0);

/// How far in front of the participant the row of servers or spaces stands, and at what height.
const ROW_DISTANCE: f32 = 1.6;
const ROW_HEIGHT: f32 = 1.35;
/// The widest the row may spread to either side, in radians, and the most room between neighbours.
const ROW_HALF_ANGLE: f32 = 1.0;
const ROW_STEP: f32 = 0.3;
/// How many objects a row holds before another row starts above it, and how far above.
const ROW_CAPACITY: usize = 12;
const ROW_RISE: f32 = 0.36;
/// The create object stands nearer and lower than the spaces, at the front, so its rings open in place without covering them.
const CREATE: Vec3 = Vec3::new(0.0, 0.95, -1.25);
/// The connected server stands to the left, shown as chosen.
const CONNECTED_ANGLE: f32 = -1.15;
const CONNECTED_DISTANCE: f32 = 1.4;
const CONNECTED_HEIGHT: f32 = 1.15;
/// A ring's smallest radius, and the arc each of its objects needs.
const RING_RADIUS: f32 = 0.3;
const RING_SPACING: f32 = 0.17;
/// The heading stands above the row.
const HEADING: Vec3 = Vec3::new(0.0, 1.85, -1.7);
/// Pointing is forgiving: an object's target is larger than its shape.
const TARGET_MARGIN: f32 = 1.5;
/// Labels are clipped to this many characters; the full text of the object pointed at is shown in the heading.
pub const LABEL_COLUMNS: usize = 22;

/// What an arrival object stands for, and so what choosing it does.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    /// A server in the list, by address.
    Server(String),
    /// The server connected to, shown as chosen; choosing it returns to the servers.
    Connected,
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
}

/// How an object should look, as far as arrival knows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Server {
        source: Source,
        /// Whether it can be joined; an incompatible server is shown but can't be chosen.
        available: bool,
    },
    /// The server connected to.
    Connected,
    Space {
        activity: Activity,
        participants: usize,
        /// A hue in `0..1` derived from the space's identifier alone, so the space looks the same to everyone and on every visit.
        hue: f32,
    },
    /// The create object, `open` while its rings are shown.
    Create { open: bool },
    /// A choice on one of the creation rings.
    Choice,
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
    // An environment ring follows when the server offers more than one environment; see `design/arrival.md`.
}

impl Step {
    pub fn title(self) -> &'static str {
        match self {
            Step::Backend => "Choose a backend",
            Step::Directory => "Choose a folder",
            Step::Session => "New or existing session",
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

/// Text anchored to a point in the world, which turns to face [`HOME`].
#[derive(Clone, Debug, PartialEq)]
pub struct Label {
    /// The top of the label, at the side given by `align`.
    pub anchor: Vec3,
    pub lines: Vec<String>,
    pub align: Align,
    /// The size of one font pixel, in metres.
    pub pixel: f32,
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
    /// Shown faintly, behind an open ring.
    pub dimmed: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Scene {
    pub objects: Vec<Object>,
    pub heading: Option<Label>,
}

impl Scene {
    /// Place `view` in the world: the servers or spaces on a row in front of [`HOME`], the create object at the front, its open ring around it, and the connected server to the left.
    pub fn layout(view: &View) -> Self {
        let mut objects = Vec::new();
        if let Some(item) = &view.connected {
            let position = on_arc(CONNECTED_ANGLE, CONNECTED_DISTANCE, CONNECTED_HEIGHT);
            objects.push(row_object(item, position, false));
        }
        let ring_open = view.ring.is_some();
        let (create, row): (Vec<_>, Vec<_>) =
            view.items.iter().partition(|item| item.key == Key::Create);
        // More than a row holds continue on rows above.
        let rows = row.len().div_ceil(ROW_CAPACITY).max(1);
        for (number, row) in row.chunks(ROW_CAPACITY).enumerate() {
            let step = if row.len() > 1 {
                ROW_STEP.min(2.0 * ROW_HALF_ANGLE / (row.len() - 1) as f32)
            } else {
                0.0
            };
            let start = -step * (row.len() - 1) as f32 / 2.0;
            let height = ROW_HEIGHT + number as f32 * ROW_RISE;
            for (index, item) in row.iter().enumerate() {
                let position = on_arc(start + step * index as f32, ROW_DISTANCE, height);
                objects.push(row_object(item, position, ring_open));
            }
        }
        if let Some(item) = create.first() {
            objects.push(row_object(item, CREATE, false));
            if let Some(ring) = &view.ring {
                objects.extend(ring_objects(ring, CREATE));
            }
        }
        let mut lines = vec![clip(&view.heading, 40)];
        lines.extend(view.status.iter().map(|status| clip(status, 40)));
        Self {
            objects,
            heading: Some(Label {
                anchor: HEADING + Vec3::Y * (rows - 1) as f32 * ROW_RISE,
                lines,
                align: Align::Centre,
                pixel: 0.0032,
            }),
        }
    }

    /// What can be pointed at: every object, a little larger than it looks.
    pub fn targets(&self) -> Vec<Target<Key>> {
        self.objects
            .iter()
            .map(|object| Target {
                key: object.key.clone(),
                shape: Shape::Sphere {
                    center: object.position,
                    radius: object.size * TARGET_MARGIN,
                },
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

fn size(kind: Kind) -> f32 {
    match kind {
        Kind::Server { .. } => 0.07,
        Kind::Connected => 0.06,
        Kind::Space { .. } => 0.08,
        Kind::Create { .. } => 0.08,
        Kind::Choice => 0.05,
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
    lines.extend(
        item.detail
            .iter()
            .map(|detail| clip(detail, LABEL_COLUMNS)),
    );
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
        },
        description: description(item),
        selected: item.selected,
        dimmed,
    }
}

/// A ring's objects around `centre`, starting at the top and going clockwise as seen from [`HOME`], each labelled on its outer side.
fn ring_objects(ring: &Ring, centre: Vec3) -> Vec<Object> {
    let count = ring.items.len();
    let radius = RING_RADIUS.max(count as f32 * RING_SPACING / std::f32::consts::TAU);
    let facing = facing(centre);
    let right = Vec3::Y.cross(facing);
    ring.items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let angle =
                std::f32::consts::FRAC_PI_2 - index as f32 * std::f32::consts::TAU / count as f32;
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
                },
                description: description(item),
                selected: item.selected,
                dimmed: false,
            }
        })
        .collect()
}

/// The horizontal direction from `point` towards [`HOME`], which labels face.
pub fn facing(point: Vec3) -> Vec3 {
    Vec3::new(HOME.x - point.x, 0.0, HOME.z - point.z)
        .try_normalize()
        .unwrap_or(Vec3::Z)
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
            connected: Some(item(Key::Connected, Kind::Connected, "Studio")),
            items,
            ring,
        }
    }

    fn ring(count: usize) -> Ring {
        Ring {
            step: Step::Backend,
            items: (0..count)
                .map(|n| item(Key::Backend(format!("b{n}")), Kind::Choice, &format!("b{n}")))
                .collect(),
        }
    }

    /// Point from [`HOME`] at `key`'s object and pull the trigger.
    fn choose(scene: &Scene, key: &Key) -> Option<Key> {
        let target = scene.object(key).unwrap().position;
        let ray = Ray::new(HOME, target - HOME).unwrap();
        let mut pointer = Pointer::default();
        pointer.update(Some(ray), Some(1.0), &scene.targets()).chosen
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
                assert_eq!(choose(&scene, &object.key).as_ref(), Some(&object.key));
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
        assert_eq!(keys, [&Key::Connected, &Key::Create]);
        let create = scene.object(&Key::Create).unwrap();
        assert_eq!(create.position, CREATE);
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
                .all(|object| object.dimmed)
        );
        assert!(!scene.object(&Key::Create).unwrap().dimmed);
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
            assert!(object.label.lines.iter().all(|line| line.len() <= LABEL_COLUMNS));
            let facing = facing(object.label.anchor);
            let towards = (HOME - object.label.anchor).with_y(0.0).normalize();
            assert!(facing.distance(towards) < 1e-5);
        }
        assert_eq!(facing(HOME), Vec3::Z);
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
}
