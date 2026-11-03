// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The hand menu: shown above the left wrist while the left palm faces the head, as a top-level menu of sections chosen by pointing, with a cue on the wrist when something waits. See `design/hand-menu.md`.
//!
//! Its items are arrival objects, so that they are drawn, labelled, pointed at and chosen as arrival's are.

use std::time::{Duration, Instant};

use glam::{Mat4, Quat, Vec3};
use protocol::{AgentInstructions, ConnectionRole, KnockRequest, KnockState};

use crate::arrival::{Align, Key, Kind, Label, Object, facing_from};
use crate::controller;
use crate::hands::Hand;

/// The menu appears once the palm has faced the head within this angle for [`HOLD`].
pub const SHOW_ANGLE: f32 = 35.0 * std::f32::consts::PI / 180.0;
/// The menu hides once the palm turns further than this from the head.
pub const HIDE_ANGLE: f32 = 55.0 * std::f32::consts::PI / 180.0;
/// How long the palm faces the head before the menu appears, so that a hand passing through the pose does not show it.
pub const HOLD: Duration = Duration::from_millis(200);

/// The OpenXR hand joints the palm is found from: the wrist and the knuckles of the index and little fingers, which are far enough apart to give a steady direction.
const WRIST: usize = 1;
const INDEX_KNUCKLE: usize = 7;
const LITTLE_KNUCKLE: usize = 22;

/// How far apart the menu's rows are, how far each reaches to the right to be pointed at, and how large their markers.
pub const ROW: f32 = 0.024;
pub const ROW_LENGTH: f32 = 0.2;
const MARKER: f32 = 0.009;
const PIXEL: f32 = 0.0011;
/// The most rows a layer of the agent's instructions takes when opened; a longer one ends with "...".
pub const LAYER_ROWS: usize = 14;
/// How much of a layer its button in the Agent section shows.
const PREVIEW: usize = 24;

/// Where a palm is and which way it faces.
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
        let index = position(INDEX_KNUCKLE)? - wrist;
        let little = position(LITTLE_KNUCKLE)? - wrist;
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

    /// The right palm of a tracked hand: for a right hand the index finger crossed with the little finger points out of the palm. See `design/wrist-transcript.md`.
    pub fn from_right_hand(hand: &Hand) -> Option<Self> {
        let position =
            |index: usize| hand.joints[index].map(|joint| Vec3::from_array(joint.position));
        let wrist = position(WRIST)?;
        let index = position(INDEX_KNUCKLE)? - wrist;
        let little = position(LITTLE_KNUCKLE)? - wrist;
        let normal = index.cross(little).try_normalize()?;
        Some(Self { wrist, normal })
    }

    /// The right palm of a controller held in the right hand: OpenXR's grip +X points into the right palm, so the palm faces along -X.
    pub fn from_right_controller(pose: &controller::Pose) -> Option<Self> {
        Self::from_left_controller(pose).map(|palm| Self {
            normal: -palm.normal,
            ..palm
        })
    }

    /// The angle, in radians, between the way the palm faces and the way to `head`.
    pub fn angle_to(&self, head: Vec3) -> Option<f32> {
        let towards = (head - self.wrist).try_normalize()?;
        Some(self.normal.dot(towards).clamp(-1.0, 1.0).acos())
    }

    /// The angle, in radians, between the way the palm faces and straight up in the scene, whose +Y is the world's up.
    pub fn angle_to_up(&self) -> f32 {
        self.normal.dot(Vec3::Y).clamp(-1.0, 1.0).acos()
    }
}

/// From the eyes to the neck, about which the head turns, in the head's space: the eyes are about 7.5 cm above it and 8 cm in front.
const EYES_TO_NECK: Vec3 = Vec3::new(0.0, -0.075, 0.0805);

/// The point that what the hands hold faces, from the head's transform with its origin between the eyes: the middle of the head, above the neck and level with the eyes.
///
/// Turning the head swings the eyes about the neck, sideways and back, by several centimetres. A panel held near the face that faced the eyes would turn against the head, and, raised above the wrist along its own up, swing sideways the other way; this point stays put as the head turns, and moves only as the head does. See `design/hand-menu.md`.
pub fn head_centre(head: Mat4) -> Vec3 {
    head.transform_point3(EYES_TO_NECK) - Vec3::Y * EYES_TO_NECK.y
}

/// The head's transform, between the eyes, turned `yaw` degrees to the left and `pitch` up about a neck at `neck`, for tests.
#[cfg(test)]
pub fn head_turned(neck: Vec3, yaw: f32, pitch: f32) -> Mat4 {
    let rotation =
        Quat::from_rotation_y(yaw.to_radians()) * Quat::from_rotation_x(pitch.to_radians());
    Mat4::from_rotation_translation(rotation, neck - rotation * EYES_TO_NECK)
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
        self.update_within(angle, SHOW_ANGLE, HIDE_ANGLE, now)
    }

    /// Update with the palm's `angle` to whatever it turns towards at `now`, or `None` when the hand is not tracked: it shows once the angle has been within `show` for [`HOLD`], and hides once it is beyond `hide`. Returns whether it is shown.
    pub fn update_within(
        &mut self,
        angle: Option<f32>,
        show: f32,
        hide: f32,
        now: Instant,
    ) -> bool {
        match angle {
            None => {
                self.shown = false;
                self.facing_since = None;
            }
            Some(angle) if self.shown => {
                if angle > hide {
                    self.shown = false;
                    self.facing_since = None;
                }
            }
            Some(angle) if angle <= show => {
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
    /// The space's title, mode and speech, this headset's settings, and leaving; shown inside a space.
    Space,
    /// Asking whether to leave the space, opened from [`Section::Space`].
    Leave,
    PeopleAndSecurity,
    /// What Tomori tells the space's agent, in layers; shown inside a space.
    Agent,
    /// One layer of the agent's instructions, opened from [`Section::Agent`].
    Layer(Layer),
}

/// A layer of the instructions Tomori gives a space's agent. See `design/spaces.md`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Layer {
    Tomori,
    Environment,
    Space,
}

impl Layer {
    fn label(self) -> &'static str {
        match self {
            Self::Tomori => "Tomori's",
            Self::Environment => "The environment's",
            Self::Space => "This space's",
        }
    }

    fn text(self, instructions: &AgentInstructions) -> &str {
        match self {
            Self::Tomori => &instructions.tomori,
            Self::Environment => &instructions.environment,
            Self::Space => &instructions.space,
        }
    }
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
    /// Open a layer of the agent's instructions.
    Layer(Layer),
    /// Remove the space's own instructions, as an owner.
    ClearInstructions,
    /// Switch the agent to the mode with this id.
    Mode(String),
    /// Switch the space's speech between brief and full, for everyone.
    Speech,
    /// Mute or unmute spoken replies on this headset.
    Mute,
    /// Turn the status pill on or off on this headset.
    Pill,
    /// Ask whether to leave the space.
    Leave,
    /// Leave the space, once asked.
    ConfirmLeave,
    /// A row that is only read, by its place.
    Text(usize),
    /// The cue on the wrist.
    Cue,
    /// The cue on the right wrist, that something waits on the transcript.
    WristCue,
}

/// How an item of the hand menu looks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MenuKind {
    Section {
        flashing: bool,
    },
    Button,
    Heading,
    Text,
    /// A participant, with a swatch of their colour.
    Person([f32; 3]),
    Cue,
    /// The right wrist's cue for a problem, in red.
    ProblemCue,
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
    /// The instructions the joined space's agent is given, or `None` when it takes none or the server does not say.
    pub instructions: Option<AgentInstructions>,
    /// The joined space, for the header and the Space section.
    pub space: Option<SpaceInfo>,
    /// The people in the space, with their colours.
    pub people: Vec<Person>,
    /// Whether spoken replies are muted on this headset.
    pub muted: bool,
    /// Whether the status pill shows on this headset.
    pub pill: bool,
    /// The item the ray points at, so that a mode's description shows beneath it.
    pub pointed: Option<MenuKey>,
}

/// The joined space, as the menu shows it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SpaceInfo {
    pub title: String,
    /// The backend and the directory's last part, such as "opencode in tomori".
    pub running: String,
    /// What the agent is doing, in a word.
    pub activity: String,
    /// The agent's modes, in order: id, name, description, and whether it is the current one.
    pub modes: Vec<(String, String, Option<String>, bool)>,
    /// The current mode's name, even when it is not one Tomori can switch to.
    pub mode: Option<String>,
    pub speech: protocol::SpeechSetting,
    /// How many are here, and away.
    pub here: String,
}

/// A participant, as People & security shows them.
#[derive(Clone, Debug, PartialEq)]
pub struct Person {
    pub name: String,
    pub colour: [f32; 3],
    pub you: bool,
    pub away: bool,
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
    /// Remove the space's own instructions to the agent.
    ClearInstructions,
    /// Switch the agent to this mode.
    Mode(String),
    /// Switch the space's speech between brief and full.
    Speech,
    /// Mute or unmute spoken replies here.
    Mute,
    /// Turn the status pill on or off here.
    Pill,
    /// Leave the space.
    Leave,
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
    // The Space and Agent sections apply only inside a space, so leaving one returns to the top level.
    let section =
        section.filter(|section| model.in_space || *section == Section::PeopleAndSecurity);
    let space = model.space.as_ref().filter(|_| model.in_space);
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
    // Inside a space, the header says where the participant is and what the agent is doing, whenever the menu opens on the top level or the Space section.
    if let Some(space) = space.filter(|_| matches!(section, None | Some(Section::Space))) {
        for line in wrap(&space.title, WIDTH).into_iter().take(2) {
            text(&mut items, MenuKind::Heading, line);
        }
        let mut status = vec![space.activity.clone()];
        status.extend(space.mode.clone());
        if space.speech == protocol::SpeechSetting::Brief {
            status.push("brief".into());
        }
        status.push(space.here.clone());
        text(&mut items, MenuKind::Text, status.join(" | "));
    }
    let section_item = |items: &mut Vec<Item>, section: Section, flashing: bool, label: &str| {
        items.push(Item {
            key: MenuKey::Section(section),
            kind: MenuKind::Section { flashing },
            text: label.into(),
        });
    };
    match section {
        None => {
            if model.in_space {
                section_item(&mut items, Section::Space, false, "Space");
                section_item(&mut items, Section::Agent, false, "Agent");
            }
            section_item(
                &mut items,
                Section::PeopleAndSecurity,
                model.waiting(),
                "People & security",
            );
        }
        Some(Section::Space) => {
            button(&mut items, MenuKey::Back, "< Back".into());
            if let Some(space) = space {
                for line in wrap(&space.title, WIDTH).into_iter().skip(2) {
                    text(&mut items, MenuKind::Text, line);
                }
                text(&mut items, MenuKind::Text, space.running.clone());
                text(&mut items, MenuKind::Heading, "Mode".into());
                if space.modes.is_empty() {
                    let mode = space.mode.as_deref().unwrap_or("none offered");
                    text(&mut items, MenuKind::Text, format!("Mode: {mode}"));
                }
                for (id, name, description, current) in &space.modes {
                    let label = if *current {
                        format!("* {name} (now)")
                    } else {
                        format!("  {name}")
                    };
                    let key = MenuKey::Mode(id.clone());
                    // Only a participant switches it; a visitor reads it.
                    if model.role.participates() {
                        button(&mut items, key.clone(), label);
                    } else {
                        text(&mut items, MenuKind::Text, label);
                    }
                    if model.pointed.as_ref() == Some(&key)
                        && let Some(description) = description
                    {
                        for line in wrap(description, WIDTH).into_iter().take(4) {
                            text(&mut items, MenuKind::Text, format!("  {line}"));
                        }
                    }
                }
                if space.speech != protocol::SpeechSetting::Unknown {
                    text(&mut items, MenuKind::Heading, "Speech, for everyone".into());
                    let said = match space.speech {
                        protocol::SpeechSetting::Brief => "Brief: switch to full",
                        _ => "Full: switch to brief",
                    };
                    if model.role.participates() {
                        button(&mut items, MenuKey::Speech, said.into());
                    } else {
                        text(
                            &mut items,
                            MenuKind::Text,
                            said.split(':').next().unwrap_or(said).into(),
                        );
                    }
                }
                text(&mut items, MenuKind::Heading, "On this headset".into());
                button(
                    &mut items,
                    MenuKey::Mute,
                    if model.muted {
                        "Spoken replies: muted".into()
                    } else {
                        "Spoken replies: on".into()
                    },
                );
                button(
                    &mut items,
                    MenuKey::Pill,
                    if model.pill {
                        "Status pill: on".into()
                    } else {
                        "Status pill: off".into()
                    },
                );
                button(&mut items, MenuKey::Leave, "Leave space".into());
            }
        }
        Some(Section::Leave) => {
            text(&mut items, MenuKind::Heading, "Leave this space?".into());
            let title = space.map_or("", |space| space.title.as_str());
            for line in wrap(title, WIDTH).into_iter().take(2) {
                text(&mut items, MenuKind::Text, line);
            }
            button(&mut items, MenuKey::ConfirmLeave, "Leave".into());
            button(&mut items, MenuKey::Back, "Stay".into());
        }
        Some(Section::Agent) => {
            button(&mut items, MenuKey::Back, "< Back".into());
            text(&mut items, MenuKind::Heading, "Instructions".into());
            match &model.instructions {
                None => text(
                    &mut items,
                    MenuKind::Text,
                    "This agent takes none from Tomori".into(),
                ),
                Some(instructions) => {
                    for layer in [Layer::Tomori, Layer::Environment, Layer::Space] {
                        let said = layer.text(instructions).trim();
                        if said.is_empty() {
                            text(
                                &mut items,
                                MenuKind::Text,
                                format!("{}: none", layer.label()),
                            );
                        } else {
                            button(
                                &mut items,
                                MenuKey::Layer(layer),
                                format!("{}: {}", layer.label(), preview(said)),
                            );
                        }
                    }
                    if model.role == ConnectionRole::Owner && !instructions.space.trim().is_empty()
                    {
                        button(
                            &mut items,
                            MenuKey::ClearInstructions,
                            "Clear this space's".into(),
                        );
                    }
                    if let Some(error) = &instructions.error {
                        for line in crate::caption::wrapped_lines(error)
                            .into_iter()
                            .take(LAYER_ROWS)
                        {
                            text(&mut items, MenuKind::Text, line);
                        }
                    }
                }
            }
        }
        Some(Section::Layer(layer)) => {
            button(&mut items, MenuKey::Back, "< Back".into());
            text(&mut items, MenuKind::Heading, layer.label().into());
            let said = model
                .instructions
                .as_ref()
                .map(|instructions| layer.text(instructions))
                .unwrap_or_default();
            let mut lines = crate::caption::wrapped_lines(said);
            if lines.len() > LAYER_ROWS {
                lines.truncate(LAYER_ROWS);
                let last = lines.last_mut().expect("truncated to at least one");
                last.truncate(crate::caption::COLUMNS - 3);
                last.push_str("...");
            }
            for line in lines {
                text(&mut items, MenuKind::Text, line);
            }
        }
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
                if model.people.is_empty() {
                    let here = if model.participants.is_empty() {
                        "nobody".to_owned()
                    } else {
                        model.participants.join(", ")
                    };
                    text(&mut items, MenuKind::Text, format!("Here: {here}"));
                } else {
                    // Each with a swatch of their colour, the key to the transcript's colours.
                    text(&mut items, MenuKind::Text, "Here:".into());
                    for person in &model.people {
                        let mut label = person.name.clone();
                        if person.you {
                            label = format!("You ({label})");
                        }
                        if person.away {
                            label.push_str(", away");
                        }
                        let key = MenuKey::Text(items.len());
                        items.push(Item {
                            key,
                            kind: MenuKind::Person(person.colour),
                            text: label,
                        });
                    }
                }
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

/// The rows of `section`, or of the top level, for `model`, as text: each row's label, with a mark for those that can be chosen, as the console prints them.
#[cfg(any(not(target_os = "android"), test))]
pub fn text(section: Option<Section>, model: &Model) -> Vec<String> {
    items(section, model)
        .into_iter()
        .filter(|item| item.key != MenuKey::Back)
        .map(|item| match item.kind {
            MenuKind::Section { .. } | MenuKind::Button => format!("[{}]", item.text),
            MenuKind::Heading => item.text.to_uppercase(),
            MenuKind::Person(_) => format!("  {}", item.text),
            _ => item.text,
        })
        .collect()
}

/// How wide the menu's wrapped rows are, in characters.
const WIDTH: usize = 32;

/// `text` wrapped to `width` characters, at words.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for word in text.split_whitespace() {
        match lines.last_mut() {
            Some(line) if line.chars().count() + 1 + word.chars().count() <= width => {
                line.push(' ');
                line.push_str(word);
            }
            _ => lines.push(word.chars().take(width).collect()),
        }
    }
    lines
}

/// The start of `text`, on one line, cut at a word with "..." when longer than [`PREVIEW`].
fn preview(text: &str) -> String {
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() <= PREVIEW {
        return line;
    }
    let cut: String = line.chars().take(PREVIEW).collect();
    let cut = cut
        .rsplit_once(' ')
        .map_or(cut.as_str(), |(start, _)| start);
    format!("{cut}...")
}

/// The hand menu's state: whether it shows, and which section is open.
#[derive(Clone, Debug, Default)]
pub struct HandMenu {
    visibility: Visibility,
    section: Option<Section>,
    /// Pinned open by Y, whatever the hand's pose, until Y or B.
    pinned: bool,
}

impl HandMenu {
    /// Update with the left palm and the head at `now`. Returns whether the menu is shown.
    pub fn update(&mut self, palm: Option<Palm>, head: Option<Vec3>, now: Instant) -> bool {
        let angle = palm.zip(head).and_then(|(palm, head)| palm.angle_to(head));
        let shown = self.visibility.update(angle, now);
        shown || self.pinned
    }

    /// Y in a space: pin the menu open at its Space section, whatever the hand's pose, or let go of it. See `design/hand-menu.md`.
    pub fn toggle_pin(&mut self) {
        self.pinned = !self.pinned;
        if self.pinned {
            self.section = Some(Section::Space);
        }
    }

    /// Let go of a pinned menu, as B does, or on leaving a space. Returns whether it was pinned.
    pub fn unpin(&mut self) -> bool {
        std::mem::take(&mut self.pinned)
    }

    pub fn pinned(&self) -> bool {
        self.pinned
    }

    /// Choose `key`: open or leave a section, or ask the conversation to act.
    pub fn choose(&mut self, key: &MenuKey) -> Option<Choice> {
        match key {
            MenuKey::Section(section) => {
                self.section = Some(*section);
                None
            }
            MenuKey::Back => {
                // A layer steps back to the Agent section, asking to leave to the Space section, and a section to the top level.
                self.section = match self.section {
                    Some(Section::Layer(_)) => Some(Section::Agent),
                    Some(Section::Leave) => Some(Section::Space),
                    _ => None,
                };
                None
            }
            MenuKey::Leave => {
                self.section = Some(Section::Leave);
                None
            }
            MenuKey::ConfirmLeave => {
                self.section = None;
                self.pinned = false;
                Some(Choice::Leave)
            }
            MenuKey::Mode(id) => Some(Choice::Mode(id.clone())),
            MenuKey::Speech => Some(Choice::Speech),
            MenuKey::Mute => Some(Choice::Mute),
            MenuKey::Pill => Some(Choice::Pill),
            MenuKey::Layer(layer) => {
                self.section = Some(Section::Layer(*layer));
                None
            }
            MenuKey::ClearInstructions => Some(Choice::ClearInstructions),
            MenuKey::Knock => Some(Choice::Knock),
            MenuKey::Admit(id) => Some(Choice::Admit(id.clone())),
            MenuKey::Decline(id) => Some(Choice::Decline(id.clone())),
            MenuKey::Pair => Some(Choice::Pair),
            MenuKey::Text(_) | MenuKey::Cue | MenuKey::WristCue => None,
        }
    }

    /// The menu's rows for `model`, laid out above the wrist of `palm` on a plane facing `head`.
    pub fn objects(&self, palm: &Palm, model: &Model, head: Vec3) -> Vec<Object> {
        let rows = items(self.section, model);
        let base = palm.wrist + palm.normal * 0.05;
        // The plane is aimed at the head from the menu's middle, so that the whole of it faces the participant; the middle depends on the aim, so it is refined once.
        let height = 0.04 + ROW * rows.len() as f32;
        let basis = |normal: Vec3| {
            let right = Vec3::Y.cross(normal).try_normalize().unwrap_or(Vec3::X);
            (normal, right, normal.cross(right))
        };
        let (_, _, up) = basis(facing_from(
            base + Vec3::Y * (height / 2.0),
            Some(head),
            true,
        ));
        let (_, right, up) = basis(facing_from(base + up * (height / 2.0), Some(head), true));
        let top = base + up * height;
        rows.into_iter()
            .enumerate()
            .map(|(index, item)| {
                let position = top - up * ROW * index as f32;
                let choosable = matches!(item.kind, MenuKind::Section { .. } | MenuKind::Button);
                Object {
                    key: Key::Menu(item.key),
                    kind: Kind::Menu(item.kind),
                    position,
                    size: MARKER,
                    label: Label {
                        anchor: position + right * (MARKER * 2.0) + up * (5.5 * PIXEL),
                        lines: vec![item.text.clone()],
                        align: Align::Left,
                        pixel: PIXEL,
                        alpha: 1.0,
                        tilt: true,
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
            tilt: true,
        },
        description: String::new(),
        selected: false,
        dim: 0.0,
        presence: 1.0,
        choosable: false,
    }
}

/// The cue on the right wrist of `palm`, while the transcript is closed and something waits on it: amber for a request, red for a problem. See `design/wrist-transcript.md`.
pub fn wrist_cue(palm: &Palm, problem: bool) -> Object {
    let mut cue = cue(palm);
    cue.key = Key::Menu(MenuKey::WristCue);
    if problem {
        cue.kind = Kind::Menu(MenuKind::ProblemCue);
    }
    cue
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::Joint;

    fn hand(wrist: Vec3, index: Vec3, little: Vec3) -> Hand {
        let mut joints = [None; crate::hands::JOINT_COUNT];
        for (place, position) in [
            (WRIST, wrist),
            (INDEX_KNUCKLE, index),
            (LITTLE_KNUCKLE, little),
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
    fn a_right_palm_mirrors_the_left() {
        // Fingers forward, palm up: for a right hand the index finger is on the right.
        let up = hand(
            Vec3::ZERO,
            Vec3::new(0.03, 0.0, -0.09),
            Vec3::new(-0.03, 0.0, -0.08),
        );
        let palm = Palm::from_right_hand(&up).unwrap();
        assert!(palm.normal.dot(Vec3::Y) > 0.99, "{:?}", palm.normal);
        // A right controller rolled the other way faces up along its grip's -X axis.
        let pose = controller::Pose {
            position: [0.2, 1.0, -0.3],
            orientation: Quat::from_rotation_z(-std::f32::consts::FRAC_PI_2).to_array(),
        };
        let palm = Palm::from_right_controller(&pose).unwrap();
        assert!(palm.normal.dot(Vec3::Y) > 0.99, "{:?}", palm.normal);
        let cue = wrist_cue(&palm, true);
        assert_eq!(cue.kind, Kind::Menu(MenuKind::ProblemCue));
        assert_eq!(cue.key, Key::Menu(MenuKey::WristCue));
        assert!(!cue.choosable);
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
    fn the_agent_section_shows_each_layer_in_a_space_and_opens_it() {
        let mut model = Model {
            role: ConnectionRole::Owner,
            ..Default::default()
        };
        // Only inside a space, where it applies.
        assert_eq!(texts(&items(None, &model)), ["People & security"]);
        assert_eq!(
            texts(&items(Some(Section::Agent), &model)),
            ["People & security"]
        );
        model.in_space = true;
        assert_eq!(
            texts(&items(None, &model)),
            ["Space", "Agent", "People & security"]
        );
        assert_eq!(
            texts(&items(Some(Section::Agent), &model)),
            [
                "< Back",
                "Instructions",
                "This agent takes none from Tomori"
            ]
        );
        let tomori = "You are Tomori, the coordinator of a shared spatial workspace. ".repeat(12);
        model.instructions = Some(AgentInstructions {
            tomori: tomori.clone(),
            environment: String::new(),
            space: "Mind the docs.".into(),
            error: Some("The agent did not take Tomori's instructions: gone".into()),
        });
        let rows = items(Some(Section::Agent), &model);
        assert_eq!(
            texts(&rows),
            [
                "< Back",
                "Instructions",
                "Tomori's: You are Tomori, the...",
                "The environment's: none",
                "This space's: Mind the docs.",
                "Clear this space's",
                "The agent did not take Tomori's",
                "instructions: gone",
            ]
        );
        assert_eq!(rows[2].key, MenuKey::Layer(Layer::Tomori));
        assert_eq!(rows[3].kind, MenuKind::Text);
        assert_eq!(rows[5].key, MenuKey::ClearInstructions);
        // Only an owner can clear the space's own.
        let collaborator = Model {
            role: ConnectionRole::Collaborator,
            ..model.clone()
        };
        assert!(
            !texts(&items(Some(Section::Agent), &collaborator)).contains(&"Clear this space's")
        );

        // A layer opens in place, wrapped and cut to fit, and steps back to the section.
        let mut menu = HandMenu {
            section: Some(Section::Agent),
            ..Default::default()
        };
        assert_eq!(menu.choose(&MenuKey::Layer(Layer::Tomori)), None);
        let rows = items(menu.section, &model);
        assert_eq!(rows[1].text, "Tomori's");
        assert_eq!(rows.len(), 2 + LAYER_ROWS);
        assert!(rows.last().unwrap().text.ends_with("..."));
        assert!(
            rows.iter()
                .all(|row| row.text.chars().count() <= crate::caption::COLUMNS)
        );
        assert_eq!(menu.choose(&MenuKey::Back), None);
        assert_eq!(menu.section, Some(Section::Agent));
        assert_eq!(
            menu.choose(&MenuKey::ClearInstructions),
            Some(Choice::ClearInstructions)
        );
        assert_eq!(menu.choose(&MenuKey::Back), None);
        assert_eq!(menu.section, None);
    }

    fn in_space(role: ConnectionRole) -> Model {
        Model {
            name: "Ada".into(),
            role,
            in_space: true,
            space: Some(SpaceInfo {
                title: "Fix the parser in the lexer before the release goes out".into(),
                running: "opencode in tomori".into(),
                activity: "running".into(),
                modes: vec![
                    (
                        "build".into(),
                        "build".into(),
                        Some("Makes changes.".into()),
                        true,
                    ),
                    (
                        "plan".into(),
                        "plan".into(),
                        Some("Plans without changing files.".into()),
                        false,
                    ),
                ],
                mode: Some("build".into()),
                speech: protocol::SpeechSetting::Brief,
                here: "2 here".into(),
            }),
            people: vec![
                Person {
                    name: "Ada".into(),
                    colour: [0.7, 0.5, 1.0],
                    you: true,
                    away: false,
                },
                Person {
                    name: "Bob".into(),
                    colour: [0.4, 0.9, 0.5],
                    you: false,
                    away: true,
                },
            ],
            pill: true,
            ..Default::default()
        }
    }

    #[test]
    fn the_header_says_where_you_are_and_what_the_agent_does() {
        let model = in_space(ConnectionRole::Owner);
        let rows = items(None, &model);
        assert_eq!(
            texts(&rows),
            [
                "Fix the parser in the lexer",
                "before the release goes out",
                "running | build | brief | 2 here",
                "Space",
                "Agent",
                "People & security",
            ]
        );
        assert_eq!(rows[0].kind, MenuKind::Heading);
        assert!(!matches!(
            rows[2].kind,
            MenuKind::Button | MenuKind::Section { .. }
        ));
        // Outside a space there is none.
        assert_eq!(
            texts(&items(None, &Model::default())),
            ["People & security"]
        );
    }

    #[test]
    fn the_space_section_switches_modes_speech_and_this_headsets_settings() {
        let mut model = in_space(ConnectionRole::Collaborator);
        let rows = items(Some(Section::Space), &model);
        assert_eq!(
            texts(&rows)[3..],
            [
                "< Back",
                "opencode in tomori",
                "Mode",
                "* build (now)",
                "  plan",
                "Speech, for everyone",
                "Brief: switch to full",
                "On this headset",
                "Spoken replies: on",
                "Status pill: on",
                "Leave space",
            ]
        );
        assert!(
            rows.iter()
                .any(|row| row.key == MenuKey::Mode("plan".into()))
        );
        // Pointing at a mode shows its description beneath it.
        model.pointed = Some(MenuKey::Mode("plan".into()));
        let shown = texts(&items(Some(Section::Space), &model))
            .iter()
            .map(|text| text.to_string())
            .collect::<Vec<_>>();
        let plan = shown.iter().position(|text| text == "  plan").unwrap();
        assert_eq!(shown[plan + 1], "  Plans without changing files.");
        let mut menu = HandMenu::default();
        assert_eq!(
            menu.choose(&MenuKey::Mode("plan".into())),
            Some(Choice::Mode("plan".into()))
        );
        assert_eq!(menu.choose(&MenuKey::Speech), Some(Choice::Speech));
        assert_eq!(menu.choose(&MenuKey::Mute), Some(Choice::Mute));
        assert_eq!(menu.choose(&MenuKey::Pill), Some(Choice::Pill));
        model.muted = true;
        model.pill = false;
        let shown = texts(&items(Some(Section::Space), &model))
            .iter()
            .map(|text| text.to_string())
            .collect::<Vec<_>>();
        assert!(shown.contains(&"Spoken replies: muted".to_owned()));
        assert!(shown.contains(&"Status pill: off".to_owned()));
        // A visitor reads the mode and speech, and cannot switch them.
        let visitor = in_space(ConnectionRole::Visitor);
        let rows = items(Some(Section::Space), &visitor);
        assert!(
            !rows
                .iter()
                .any(|row| matches!(row.key, MenuKey::Mode(_) | MenuKey::Speech))
        );
        assert!(texts(&rows).contains(&"* build (now)"));
        assert!(texts(&rows).contains(&"Brief"));
    }

    #[test]
    fn leaving_from_the_menu_asks_first() {
        let model = in_space(ConnectionRole::Owner);
        let mut menu = HandMenu::default();
        menu.choose(&MenuKey::Section(Section::Space));
        assert_eq!(menu.choose(&MenuKey::Leave), None);
        let rows = items(menu.section, &model);
        assert_eq!(
            texts(&rows),
            [
                "Leave this space?",
                "Fix the parser in the lexer",
                "before the release goes out",
                "Leave",
                "Stay",
            ]
        );
        // Stay steps back to the Space section without leaving.
        assert_eq!(menu.choose(&MenuKey::Back), None);
        assert_eq!(menu.section, Some(Section::Space));
        menu.choose(&MenuKey::Leave);
        assert_eq!(menu.choose(&MenuKey::ConfirmLeave), Some(Choice::Leave));
        assert_eq!(menu.section, None);
    }

    #[test]
    fn y_pins_the_menu_open_at_the_space_section_until_y_or_b() {
        let start = Instant::now();
        let mut menu = HandMenu::default();
        assert!(!menu.update(None, None, start));
        menu.toggle_pin();
        assert!(menu.pinned());
        assert_eq!(menu.section, Some(Section::Space));
        // Shown whatever the hand's pose, or with no hand at all.
        assert!(menu.update(None, None, start));
        menu.toggle_pin();
        assert!(!menu.update(None, None, start));
        menu.toggle_pin();
        assert!(menu.unpin());
        assert!(!menu.unpin());
        assert!(!menu.update(None, None, start));
    }

    #[test]
    fn people_have_swatches_with_you_and_away_marked() {
        let model = in_space(ConnectionRole::Owner);
        let rows = items(Some(Section::PeopleAndSecurity), &model);
        let people: Vec<_> = rows
            .iter()
            .filter(|row| matches!(row.kind, MenuKind::Person(_)))
            .collect();
        assert_eq!(people.len(), 2);
        assert_eq!(people[0].text, "You (Ada)");
        assert_eq!(people[0].kind, MenuKind::Person([0.7, 0.5, 1.0]));
        assert_eq!(people[1].text, "Bob, away");
        // The console's text marks what can be chosen.
        let shown = text(Some(Section::Space), &model);
        assert!(shown.contains(&"[Leave space]".to_owned()));
        assert!(shown.contains(&"MODE".to_owned()));
        assert!(text(Some(Section::PeopleAndSecurity), &model).contains(&"  Bob, away".to_owned()));
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
        let objects = menu.objects(&palm, &model, Vec3::new(0.0, 1.6, 0.0));
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
    fn the_menu_lies_on_a_plane_facing_the_head() {
        let palm = Palm {
            wrist: Vec3::new(-0.15, 1.0, -0.3),
            normal: Vec3::new(0.2, 0.8, 0.4).normalize(),
        };
        let head = Vec3::new(0.1, 1.65, 0.05);
        let model = Model {
            role: ConnectionRole::Owner,
            ..Default::default()
        };
        let menu = HandMenu {
            section: Some(Section::PeopleAndSecurity),
            ..Default::default()
        };
        let objects = menu.objects(&palm, &model, head);
        let middle = objects[objects.len() / 2].position;
        let towards = (head - middle).normalize();
        // Rows run down the plane, across the line of sight.
        let down = (objects[1].position - objects[0].position).normalize();
        assert!(down.dot(towards).abs() < 0.06, "{down:?} {towards:?}");
        assert!(objects.iter().all(|object| object.label.tilt));
    }

    #[test]
    fn the_middle_of_the_head_stays_put_as_the_head_turns() {
        let neck = Vec3::new(0.1, 1.5, 0.05);
        let level = head_turned(neck, 0.0, 0.0);
        let eyes = level.transform_point3(Vec3::ZERO);
        let centre = head_centre(level);
        // Level with the eyes, and behind them.
        assert!((centre.y - eyes.y).abs() < 1e-5);
        assert!((centre - eyes).dot(Vec3::Z) > 0.07);
        for (yaw, pitch) in [(-45.0, 0.0), (45.0, 0.0), (30.0, -40.0), (-60.0, 20.0)] {
            let head = head_turned(neck, yaw, pitch);
            // The eyes swing by centimetres...
            assert!(head.transform_point3(Vec3::ZERO).distance(eyes) > 0.02);
            // ...but the middle of the head does not.
            assert!(head_centre(head).distance(centre) < 1e-5, "{yaw} {pitch}");
        }
    }

    #[test]
    fn the_menu_stays_on_the_wrist_as_the_head_turns() {
        let palm = Palm {
            wrist: Vec3::new(-0.15, 1.2, -0.3),
            normal: Vec3::new(0.2, 0.6, 0.6).normalize(),
        };
        let model = Model {
            role: ConnectionRole::Owner,
            ..Default::default()
        };
        let menu = HandMenu {
            section: Some(Section::PeopleAndSecurity),
            ..Default::default()
        };
        let neck = Vec3::new(0.0, 1.5, 0.05);
        let place = |yaw: f32| {
            menu.objects(&palm, &model, head_centre(head_turned(neck, yaw, -30.0)))
                .into_iter()
                .map(|object| object.position)
                .collect::<Vec<_>>()
        };
        let ahead = place(0.0);
        for yaw in [-45.0, 45.0] {
            for (turned, ahead) in place(yaw).iter().zip(&ahead) {
                assert!(turned.distance(*ahead) < 1e-5, "{yaw}: {turned} {ahead}");
            }
        }
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
