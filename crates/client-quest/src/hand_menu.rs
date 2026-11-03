// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The hand menu: shown above the left wrist while the left palm faces the head, as a top-level menu of sections chosen by pointing, with a cue on the wrist when something waits. See `design/hand-menu.md`.
//!
//! Its items are arrival objects, so that they are drawn, labelled, pointed at and chosen as arrival's are.

use std::time::{Duration, Instant};

use glam::{Quat, Vec3};
use protocol::{ConnectionRole, KnockRequest, KnockState};

use crate::arrival::{Align, Key, Kind, Label, Object, facing};
use crate::controller;
use crate::hands::Hand;

/// The menu appears once the palm has faced the head within this angle for [`HOLD`].
pub const SHOW_ANGLE: f32 = 35.0 * std::f32::consts::PI / 180.0;
/// The menu hides once the palm turns further than this from the head.
pub const HIDE_ANGLE: f32 = 55.0 * std::f32::consts::PI / 180.0;
/// How long the palm faces the head before the menu appears, so that a hand passing through the pose does not show it.
pub const HOLD: Duration = Duration::from_millis(200);

/// The OpenXR hand joints the palm is found from.
const WRIST: usize = 1;
const INDEX_METACARPAL: usize = 6;
const LITTLE_METACARPAL: usize = 21;

/// How far apart the menu's rows are, and how large their markers.
const ROW: f32 = 0.032;
const MARKER: f32 = 0.009;
const PIXEL: f32 = 0.0011;

/// Where the left palm is and which way it faces.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palm {
    pub wrist: Vec3,
    /// Out of the palm, unit length.
    pub normal: Vec3,
}

impl Palm {
    /// The left palm of a tracked hand, from its wrist and the bases of its index and little fingers.
    pub fn from_left_hand(hand: &Hand) -> Option<Self> {
        let position =
            |index: usize| hand.joints[index].map(|joint| Vec3::from_array(joint.position));
        let wrist = position(WRIST)?;
        let index = position(INDEX_METACARPAL)? - wrist;
        let little = position(LITTLE_METACARPAL)? - wrist;
        // For a left hand, the little finger crossed with the index finger points out of the palm.
        let normal = little.cross(index).try_normalize()?;
        Some(Self { wrist, normal })
    }

    /// The left palm of a controller held in the left hand: its grip pose's +X axis points away from the palm.
    pub fn from_left_controller(pose: &controller::Pose) -> Option<Self> {
        let orientation = Quat::from_array(pose.orientation);
        if orientation.length_squared().is_nan() || orientation.length_squared() <= 1e-6 {
            return None;
        }
        let orientation = orientation.normalize();
        Some(Self {
            wrist: Vec3::from_array(pose.position),
            normal: orientation * Vec3::X,
        })
    }

    /// The angle, in radians, between the way the palm faces and the way to `head`.
    pub fn angle_to(&self, head: Vec3) -> Option<f32> {
        let towards = (head - self.wrist).try_normalize()?;
        Some(self.normal.dot(towards).clamp(-1.0, 1.0).acos())
    }
}

/// Whether the menu is shown, with hysteresis in angle and time.
#[derive(Clone, Debug, Default)]
pub struct Visibility {
    shown: bool,
    facing_since: Option<Instant>,
}

impl Visibility {
    /// Update with the palm's angle to the head at `now`, or `None` when the hand is not tracked. Returns whether the menu is shown.
    pub fn update(&mut self, angle: Option<f32>, now: Instant) -> bool {
        match angle {
            None => {
                self.shown = false;
                self.facing_since = None;
            }
            Some(angle) if self.shown => {
                if angle > HIDE_ANGLE {
                    self.shown = false;
                    self.facing_since = None;
                }
            }
            Some(angle) if angle <= SHOW_ANGLE => {
                let since = *self.facing_since.get_or_insert(now);
                self.shown = now.saturating_duration_since(since) >= HOLD;
            }
            Some(_) => self.facing_since = None,
        }
        self.shown
    }
}

/// A section of the hand menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Section {
    PeopleAndSecurity,
}

/// What an item of the hand menu stands for.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum MenuKey {
    Section(Section),
    Back,
    Knock,
    Admit(String),
    Decline(String),
    Pair,
    /// A row that is only read, by its place.
    Text(usize),
    /// The cue on the wrist.
    Cue,
}

/// How an item of the hand menu looks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuKind {
    Section { flashing: bool },
    Button,
    Heading,
    Text,
    Cue,
}

/// What the menu shows, from the conversation.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Model {
    pub name: String,
    pub role: ConnectionRole,
    pub in_space: bool,
    pub participants: Vec<String>,
    pub visitors: usize,
    pub knocks: Vec<KnockRequest>,
    pub can_knock: bool,
    pub knock: Option<KnockState>,
    /// The pairing object's label while pairing is offered: "Pair", or the code while pairing.
    pub pairing: Option<String>,
    pub notice: Option<String>,
}

impl Model {
    /// Whether something waits for this participant, which the cue and the flashing section show.
    pub fn waiting(&self) -> bool {
        !self.knocks.is_empty()
    }
}

/// What choosing an item asks of the conversation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Choice {
    Knock,
    Admit(String),
    Decline(String),
    Pair,
}

/// One row of the menu.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub key: MenuKey,
    pub kind: MenuKind,
    pub text: String,
}

fn role_label(role: ConnectionRole) -> &'static str {
    match role {
        ConnectionRole::Owner => "owner",
        ConnectionRole::Collaborator => "collaborator",
        ConnectionRole::Visitor => "visitor",
        ConnectionRole::Pairing => "pairing",
        ConnectionRole::Unknown => "connecting",
    }
}

/// The rows of `section`, or of the top level, for `model`.
pub fn items(section: Option<Section>, model: &Model) -> Vec<Item> {
    let mut items = Vec::new();
    let text = |items: &mut Vec<Item>, kind: MenuKind, text: String| {
        let key = MenuKey::Text(items.len());
        items.push(Item { key, kind, text });
    };
    let button = |items: &mut Vec<Item>, key: MenuKey, text: String| {
        items.push(Item {
            key,
            kind: MenuKind::Button,
            text,
        });
    };
    match section {
        None => items.push(Item {
            key: MenuKey::Section(Section::PeopleAndSecurity),
            kind: MenuKind::Section {
                flashing: model.waiting(),
            },
            text: "People & security".into(),
        }),
        Some(Section::PeopleAndSecurity) => {
            button(&mut items, MenuKey::Back, "< Back".into());
            text(&mut items, MenuKind::Heading, "Requests".into());
            let mut requests = false;
            if model.role.participates() {
                for knock in &model.knocks {
                    button(
                        &mut items,
                        MenuKey::Admit(knock.id.clone()),
                        format!("Let {} in", knock.name),
                    );
                    button(
                        &mut items,
                        MenuKey::Decline(knock.id.clone()),
                        format!("Turn {} away", knock.name),
                    );
                    requests = true;
                }
            }
            if model.can_knock {
                button(&mut items, MenuKey::Knock, "Knock to take part".into());
                requests = true;
            }
            if let Some(state) = model.knock {
                let said = match state {
                    KnockState::Pending => "Knocked; waiting to be let in",
                    KnockState::Declined => "Your knock was declined",
                    KnockState::Expired => "Nobody answered your knock",
                    KnockState::Withdrawn | KnockState::Unknown => "Your knock was withdrawn",
                };
                text(&mut items, MenuKind::Text, said.into());
                requests = true;
            }
            if let Some(pairing) = &model.pairing {
                button(&mut items, MenuKey::Pair, pairing.clone());
                requests = true;
            }
            if !requests {
                text(&mut items, MenuKind::Text, "Nothing waiting".into());
            }
            text(&mut items, MenuKind::Heading, "Roles & security".into());
            text(
                &mut items,
                MenuKind::Text,
                format!("You: {} ({})", model.name, role_label(model.role)),
            );
            if model.in_space {
                let here = if model.participants.is_empty() {
                    "nobody".to_owned()
                } else {
                    model.participants.join(", ")
                };
                text(&mut items, MenuKind::Text, format!("Here: {here}"));
                text(
                    &mut items,
                    MenuKind::Text,
                    format!("Visitors watching: {}", model.visitors),
                );
            }
            if let Some(notice) = &model.notice {
                text(&mut items, MenuKind::Text, notice.clone());
            }
        }
    }
    items
}

/// The hand menu's state: whether it shows, and which section is open.
#[derive(Clone, Debug, Default)]
pub struct HandMenu {
    visibility: Visibility,
    section: Option<Section>,
}

impl HandMenu {
    /// Update with the left palm and the head at `now`. Returns whether the menu is shown.
    pub fn update(&mut self, palm: Option<Palm>, head: Option<Vec3>, now: Instant) -> bool {
        let angle = palm.zip(head).and_then(|(palm, head)| palm.angle_to(head));
        self.visibility.update(angle, now)
    }

    /// Choose `key`: open or leave a section, or ask the conversation to act.
    pub fn choose(&mut self, key: &MenuKey) -> Option<Choice> {
        match key {
            MenuKey::Section(section) => {
                self.section = Some(*section);
                None
            }
            MenuKey::Back => {
                self.section = None;
                None
            }
            MenuKey::Knock => Some(Choice::Knock),
            MenuKey::Admit(id) => Some(Choice::Admit(id.clone())),
            MenuKey::Decline(id) => Some(Choice::Decline(id.clone())),
            MenuKey::Pair => Some(Choice::Pair),
            MenuKey::Text(_) | MenuKey::Cue => None,
        }
    }

    /// The menu's rows for `model`, laid out above the wrist of `palm`, facing the participant.
    pub fn objects(&self, palm: &Palm, model: &Model) -> Vec<Object> {
        let rows = items(self.section, model);
        let top = palm.wrist + palm.normal * 0.05 + Vec3::Y * (0.06 + ROW * rows.len() as f32);
        let right = Vec3::Y.cross(facing(top)).normalize_or_zero();
        rows.into_iter()
            .enumerate()
            .map(|(index, item)| {
                let position = top - Vec3::Y * ROW * index as f32;
                let choosable = matches!(item.kind, MenuKind::Section { .. } | MenuKind::Button);
                Object {
                    key: Key::Menu(item.key),
                    kind: Kind::Menu(item.kind),
                    position,
                    size: MARKER,
                    label: Label {
                        anchor: position + right * (MARKER * 2.0) + Vec3::Y * (5.5 * PIXEL),
                        lines: vec![item.text.clone()],
                        align: Align::Left,
                        pixel: PIXEL,
                        alpha: 1.0,
                    },
                    description: item.text,
                    selected: false,
                    dim: 0.0,
                    presence: 1.0,
                    choosable,
                }
            })
            .collect()
    }
}

/// The cue on the wrist of `palm`, shown while the menu is closed and something waits.
pub fn cue(palm: &Palm) -> Object {
    Object {
        key: Key::Menu(MenuKey::Cue),
        kind: Kind::Menu(MenuKind::Cue),
        position: palm.wrist + palm.normal * 0.02,
        size: 0.008,
        label: Label {
            anchor: palm.wrist,
            lines: Vec::new(),
            align: Align::Centre,
            pixel: PIXEL,
            alpha: 0.0,
        },
        description: String::new(),
        selected: false,
        dim: 0.0,
        presence: 1.0,
        choosable: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::Joint;

    fn hand(wrist: Vec3, index: Vec3, little: Vec3) -> Hand {
        let mut joints = [None; crate::hands::JOINT_COUNT];
        for (place, position) in [
            (WRIST, wrist),
            (INDEX_METACARPAL, index),
            (LITTLE_METACARPAL, little),
        ] {
            joints[place] = Some(Joint {
                position: position.to_array(),
                radius: 0.01,
            });
        }
        Hand { joints }
    }

    #[test]
    fn a_left_palm_turned_up_faces_up() {
        // Fingers forward, palm up: for a left hand the index finger is on the left.
        let up = hand(
            Vec3::ZERO,
            Vec3::new(-0.03, 0.0, -0.09),
            Vec3::new(0.03, 0.0, -0.08),
        );
        let palm = Palm::from_left_hand(&up).unwrap();
        assert!(palm.normal.dot(Vec3::Y) > 0.99, "{:?}", palm.normal);
        assert!(palm.angle_to(Vec3::new(0.0, 0.5, 0.0)).unwrap() < 0.01);
        // A hand missing a joint has no palm.
        let mut missing = up;
        missing.joints[WRIST] = None;
        assert_eq!(Palm::from_left_hand(&missing), None);
    }

    #[test]
    fn a_left_controller_palm_faces_along_its_grip_x_axis() {
        let pose = controller::Pose {
            position: [0.0, 1.0, -0.3],
            orientation: Quat::from_rotation_z(std::f32::consts::FRAC_PI_2).to_array(),
        };
        let palm = Palm::from_left_controller(&pose).unwrap();
        assert!(palm.normal.dot(Vec3::Y) > 0.99, "{:?}", palm.normal);
        let degenerate = controller::Pose {
            position: [0.0; 3],
            orientation: [0.0; 4],
        };
        assert_eq!(Palm::from_left_controller(&degenerate), None);
    }

    #[test]
    fn the_menu_shows_after_a_hold_and_hides_past_the_outer_angle() {
        let start = Instant::now();
        let mut visibility = Visibility::default();
        let degrees = |value: f32| Some(value.to_radians());
        assert!(!visibility.update(degrees(20.0), start));
        assert!(!visibility.update(degrees(20.0), start + HOLD / 2));
        assert!(visibility.update(degrees(20.0), start + HOLD));
        // Shown until past the outer angle, not the inner one.
        assert!(visibility.update(degrees(45.0), start + HOLD * 2));
        assert!(!visibility.update(degrees(60.0), start + HOLD * 3));
        // A hand passing through the pose does not show it.
        assert!(!visibility.update(degrees(20.0), start + HOLD * 4));
        assert!(!visibility.update(degrees(50.0), start + HOLD * 5));
        assert!(!visibility.update(degrees(20.0), start + HOLD * 6));
        assert!(!visibility.update(degrees(20.0), start + HOLD * 6 + HOLD / 2));
        // Losing the hand hides it.
        assert!(visibility.update(degrees(20.0), start + HOLD * 7));
        assert!(!visibility.update(None, start + HOLD * 8));
    }

    fn texts(items: &[Item]) -> Vec<&str> {
        items.iter().map(|item| item.text.as_str()).collect()
    }

    #[test]
    fn the_top_level_flashes_people_and_security_while_someone_knocks() {
        let mut model = Model::default();
        let top = items(None, &model);
        assert_eq!(texts(&top), ["People & security"]);
        assert_eq!(top[0].kind, MenuKind::Section { flashing: false });
        model.knocks.push(KnockRequest {
            id: "knock-1".into(),
            name: "Vic".into(),
        });
        assert_eq!(
            items(None, &model)[0].kind,
            MenuKind::Section { flashing: true }
        );
    }

    #[test]
    fn people_and_security_holds_requests_apart_from_roles() {
        let owner = Model {
            name: "Ada".into(),
            role: ConnectionRole::Owner,
            in_space: true,
            participants: vec!["Ada".into(), "Bob".into()],
            visitors: 2,
            knocks: vec![KnockRequest {
                id: "knock-1".into(),
                name: "Vic".into(),
            }],
            ..Default::default()
        };
        let rows = items(Some(Section::PeopleAndSecurity), &owner);
        assert_eq!(
            texts(&rows),
            [
                "< Back",
                "Requests",
                "Let Vic in",
                "Turn Vic away",
                "Roles & security",
                "You: Ada (owner)",
                "Here: Ada, Bob",
                "Visitors watching: 2",
            ]
        );
        assert_eq!(rows[2].key, MenuKey::Admit("knock-1".into()));
        assert_eq!(rows[3].key, MenuKey::Decline("knock-1".into()));

        let visitor = Model {
            name: "Vic".into(),
            role: ConnectionRole::Visitor,
            in_space: true,
            can_knock: true,
            pairing: Some("Pair".into()),
            notice: Some("This server's identity has changed.".into()),
            knocks: owner.knocks.clone(),
            ..Default::default()
        };
        let rows = items(Some(Section::PeopleAndSecurity), &visitor);
        assert_eq!(
            texts(&rows),
            [
                "< Back",
                "Requests",
                "Knock to take part",
                "Pair",
                "Roles & security",
                "You: Vic (visitor)",
                "Here: nobody",
                "Visitors watching: 0",
                "This server's identity has changed.",
            ]
        );
        let waiting = Model {
            role: ConnectionRole::Visitor,
            knock: Some(KnockState::Pending),
            ..Default::default()
        };
        assert!(
            texts(&items(Some(Section::PeopleAndSecurity), &waiting))
                .contains(&"Knocked; waiting to be let in")
        );
        let idle = Model {
            role: ConnectionRole::Owner,
            ..Default::default()
        };
        assert!(
            texts(&items(Some(Section::PeopleAndSecurity), &idle)).contains(&"Nothing waiting")
        );
    }

    #[test]
    fn choosing_opens_sections_and_asks_for_actions() {
        let mut menu = HandMenu::default();
        assert_eq!(
            menu.choose(&MenuKey::Section(Section::PeopleAndSecurity)),
            None
        );
        assert_eq!(menu.section, Some(Section::PeopleAndSecurity));
        assert_eq!(menu.choose(&MenuKey::Knock), Some(Choice::Knock));
        assert_eq!(
            menu.choose(&MenuKey::Admit("k".into())),
            Some(Choice::Admit("k".into()))
        );
        assert_eq!(
            menu.choose(&MenuKey::Decline("k".into())),
            Some(Choice::Decline("k".into()))
        );
        assert_eq!(menu.choose(&MenuKey::Pair), Some(Choice::Pair));
        assert_eq!(menu.choose(&MenuKey::Text(3)), None);
        assert_eq!(menu.choose(&MenuKey::Back), None);
        assert_eq!(menu.section, None);
    }

    #[test]
    fn the_rows_stand_above_the_wrist_and_only_buttons_are_chosen() {
        let palm = Palm {
            wrist: Vec3::new(-0.2, 1.1, -0.35),
            normal: Vec3::Y,
        };
        let menu = HandMenu {
            section: Some(Section::PeopleAndSecurity),
            ..Default::default()
        };
        let model = Model {
            role: ConnectionRole::Owner,
            ..Default::default()
        };
        let objects = menu.objects(&palm, &model);
        assert!(
            objects
                .iter()
                .all(|object| object.position.y > palm.wrist.y)
        );
        // Rows go down the menu in order.
        assert!(
            objects
                .windows(2)
                .all(|pair| pair[0].position.y > pair[1].position.y)
        );
        let chosen: Vec<bool> = objects.iter().map(|object| object.choosable).collect();
        assert!(chosen[0], "Back");
        assert!(!chosen[1], "Requests heading");
        let cue = cue(&palm);
        assert!(!cue.choosable);
        assert_eq!(cue.kind, Kind::Menu(MenuKind::Cue));
    }

    #[test]
    fn showing_follows_the_palm_and_the_head() {
        let start = Instant::now();
        let mut menu = HandMenu::default();
        let palm = Palm {
            wrist: Vec3::new(0.0, 1.0, -0.3),
            normal: Vec3::new(0.0, 0.6, 0.3).normalize(),
        };
        let head = Vec3::new(0.0, 1.6, 0.0);
        assert!(!menu.update(Some(palm), Some(head), start));
        assert!(menu.update(Some(palm), Some(head), start + HOLD));
        assert!(!menu.update(None, Some(head), start + HOLD * 2));
        for role in [
            ConnectionRole::Collaborator,
            ConnectionRole::Pairing,
            ConnectionRole::Unknown,
        ] {
            assert!(!role_label(role).is_empty());
        }
    }
}
