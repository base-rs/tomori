// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The hand menu: shown above the left wrist while the left palm is turned up, as a top-level menu of sections chosen by pointing, with a cue on the wrist when something waits. See `design/hand-menu.md`.
//!
//! Its items are arrival objects, so that they are drawn, labelled, pointed at and chosen as arrival's are.
use std::time::{Duration, Instant};

use glam::{Mat4, Quat, Vec2, Vec3};
use tomori_protocol::{AgentInstructions, ConnectionRole, KnockRequest, KnockState};

use crate::arrival::{Align, Key, Kind, Label, Object, facing_from};
use crate::controller;
use crate::hands::Hand;
use crate::panel::{Area, Plane, Rect};
use crate::ui_text::{CELL_WIDTH, Span, Style, clip, cut, drawn, lines, plain};
use tomori_client::feedback::Hand as Side;

/// Shown after a visitor's display name wherever others see it, as on a knock: the name is the one their device suggested or they chose, with no token behind it, so nothing vouches for it. See `design/security.md`.
pub const VISITOR: &str = " (visitor)";

/// A palm shows what its hand holds once it has faced within this angle of straight up for [`HOLD`], whichever way it leans and wherever the head is, so that any palm-up pose shows it. A palm in the neutral grip faces inwards, 90° from up. See `design/hand-menu.md`.
pub const SHOW_ANGLE: f32 = 45.0 * std::f32::consts::PI / 180.0;
/// It hides once the palm has turned further than this from straight up, so that it does not flicker at the edge.
pub const HIDE_ANGLE: f32 = 55.0 * std::f32::consts::PI / 180.0;
/// How long the palm faces up before it shows, so that a hand passing through the pose does not show it.
pub const HOLD: Duration = Duration::from_millis(200);

/// The OpenXR hand joints the palm is found from: the wrist and the knuckles of the index and little fingers, which are far enough apart to give a steady direction.
const WRIST: usize = 1;
const INDEX_KNUCKLE: usize = 7;
const LITTLE_KNUCKLE: usize = 22;

/// How far apart the menu's rows are, and how large their markers.
pub const ROW: f32 = 0.024;
pub const MARKER: f32 = 0.009;
/// The size of a font pixel of the menu's labels, in metres.
pub const PIXEL: f32 = 0.0011;
/// How far the menu reaches to the right of the middle of its rows' markers, in metres. A row's label starts two markers' widths along, and its background, sized to its text, ends two font pixels past its last cell.
pub const PANEL: f32 = 0.42;
/// How many characters every row of the menu holds, at most: as many cells of the font as fit in [`PANEL`]. Rows are wrapped or cut to it by [`crate::ui_text`], and a row's names, titles and labels are cut to leave room for the rest of it.
pub const WIDTH: usize = ((PANEL - 2.0 * MARKER) / PIXEL - 2.0) as usize / CELL_WIDTH;
/// The most rows a layer of the agent's instructions takes when opened; a longer one ends with "…".
pub const LAYER_ROWS: usize = 14;
/// The most rows a notice takes, and the description of the mode pointed at.
const NOTICE_ROWS: usize = 3;
const DESCRIPTION_ROWS: usize = 4;

/// The sign that turns the geometry of the hand on `side` into the way its palm faces. OpenXR's grip +X axis is normal to the palm, pointing away from the left palm and into the right one; and the little finger's knuckle crossed with the index finger's, from the wrist, points out of the left palm and into the right one.
fn palm_sign(side: Side) -> f32 {
    match side {
        Side::Left => 1.0,
        Side::Right => -1.0,
    }
}

/// Where a palm is and which way it faces.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palm {
    pub wrist: Vec3,
    /// Out of the palm, unit length.
    pub normal: Vec3,
}

impl Palm {
    /// The palm of a tracked hand on `side`, from its wrist and the bases of its index and little fingers.
    pub fn from_hand(side: Side, hand: &Hand) -> Option<Self> {
        let position =
            |index: usize| hand.joints[index].map(|joint| Vec3::from_array(joint.position));
        let wrist = position(WRIST)?;
        let index = position(INDEX_KNUCKLE)? - wrist;
        let little = position(LITTLE_KNUCKLE)? - wrist;
        let normal = (little.cross(index) * palm_sign(side)).try_normalize()?;
        Some(Self { wrist, normal })
    }

    /// The palm of a controller held in the hand on `side`, along its grip pose's X axis.
    pub fn from_controller(side: Side, pose: &controller::Pose) -> Option<Self> {
        let orientation = Quat::from_array(pose.orientation);
        if orientation.length_squared().is_nan() || orientation.length_squared() <= 1e-6 {
            return None;
        }
        let orientation = orientation.normalize();
        Some(Self {
            wrist: Vec3::from_array(pose.position),
            normal: orientation * Vec3::X * palm_sign(side),
        })
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

/// The gesture both hands share, turning the palm up: whether what the hand holds is shown, with hysteresis in angle and time. It shows once the palm has faced within [`SHOW_ANGLE`] of straight up for [`HOLD`], and hides once it is beyond [`HIDE_ANGLE`] or the hand is no longer tracked. Only the palm counts, not where the head is. Each hand's own reasons not to show are its caller's. See `design/hand-menu.md` and `design/wrist-transcript.md`.
#[derive(Clone, Debug, Default)]
pub struct PalmUp {
    shown: bool,
    up_since: Option<Instant>,
}

impl PalmUp {
    /// Update with `palm` at `now`, or `None` when the hand is not tracked or must not show. Returns whether it is shown.
    pub fn update(&mut self, palm: Option<Palm>, now: Instant) -> bool {
        match palm.map(|palm| palm.angle_to_up()) {
            None => {
                self.shown = false;
                self.up_since = None;
            }
            Some(angle) if self.shown => {
                if angle > HIDE_ANGLE {
                    self.shown = false;
                    self.up_since = None;
                }
            }
            Some(angle) if angle <= SHOW_ANGLE => {
                let since = *self.up_since.get_or_insert(now);
                self.shown = now.saturating_duration_since(since) >= HOLD;
            }
            Some(_) => self.up_since = None,
        }
        self.shown
    }
}

/// A section of the hand menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Section {
    /// The space's title, mode, speech and environment, this headset's settings, and leaving; shown inside a space.
    Space,
    /// Asking whether to leave the space, opened from [`Section::Space`].
    Leave,
    /// Asking whether to delete the space, for an owner, opened from [`Section::Space`], with whether its agent session is to be deleted too, which starts off. See `design/hand-menu.md`.
    Delete {
        session: bool,
    },
    /// Asking again, before deleting the agent session with the space, since that cannot be undone; opened from [`Section::Delete`].
    DeleteSession,
    PeopleAndSecurity,
    /// What Tomori tells the space's agent, in layers; shown inside a space.
    Agent,
    /// One layer of the agent's instructions, opened from [`Section::Agent`].
    Layer(Layer),
    /// This device's Feedback setting: which sounds and vibrations play. See `design/feedback.md`.
    Sound,
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
    /// Say a new display name with the next press of X, or not after all.
    Rename,
    /// Open a layer of the agent's instructions.
    Layer(Layer),
    /// Remove the space's own instructions, as an owner.
    ClearInstructions,
    /// Switch the agent to the mode with this id.
    Mode(String),
    /// Switch the space's speech between brief and full, for everyone.
    Speech,
    /// Change the space's environment to the one with this id, for everyone, as an owner.
    Environment(String),
    /// Mute or unmute spoken replies on this headset.
    Mute,
    /// Turn the status pill on or off on this headset.
    Pill,
    /// Turn travel off on this headset, staying room scale, or on again.
    Travel,
    /// Add a projection wall where the participant stands, for everyone, as an owner.
    AddWall,
    /// Remove the added projection wall nearest the participant, for everyone, as an owner.
    RemoveWall,
    /// Ask whether to leave the space.
    Leave,
    /// Leave the space, once asked.
    ConfirmLeave,
    /// Ask whether to delete the space, as an owner.
    Delete,
    /// Turn deleting the agent session with the space on or off, while asked whether to delete it.
    DeleteSession,
    /// Delete the space, once asked, or, with its session, ask again.
    ConfirmDelete,
    /// Delete the space and its session, once asked again.
    ConfirmDeleteSession,
    /// Step the Feedback setting's sounds or vibration to the next level.
    Sounds,
    Vibration,
    /// Turn mono audio on or off.
    Mono,
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
    /// The participant's display name, as the server knows them.
    pub name: String,
    /// Whether they can say a new one: the server renames, and they take part in the space they are in.
    pub renames: bool,
    /// Whether the next press of X says their new name.
    pub naming: bool,
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
    /// Whether the participant travels on this headset, where the space's environment allows travel; `None` where it does not, which offers no setting. See `design/locomotion.md`.
    pub travel: Option<bool>,
    /// The item the ray points at, so that the description of the mode pointed at shows beneath the modes. It changes what the rows say, never how many there are, so that the menu's layout does not follow the ray.
    pub pointed: Option<MenuKey>,
    /// This device's Feedback setting.
    pub feedback: tomori_client::feedback::Settings,
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
    pub speech: tomori_protocol::SpeechSetting,
    /// The name of the space's environment, or `None` from an older server, which does not name one.
    pub environment: Option<String>,
    /// The environments an owner can change the space to, in order: id, name, and whether it is the current one. Empty
    /// for anyone else, and from a server that does not change environments. See `design/environments.md`.
    pub environments: Vec<(String, String, bool)>,
    /// How many are here, and away.
    pub here: String,
    /// For an owner in a space the server projects in, whether an added wall can be removed; `None` for anyone else,
    /// who does not manage the walls. See `design/projection.md`.
    pub walls: Option<bool>,
    /// For an owner on a server that deletes spaces, whether its agent session can be deleted with it; `None` for
    /// anyone else, who cannot delete it. See `design/spaces.md`.
    pub deletion: Option<bool>,
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
    /// Say a new display name with the next press of X, or not after all.
    Rename,
    /// Remove the space's own instructions to the agent.
    ClearInstructions,
    /// Switch the agent to this mode.
    Mode(String),
    /// Switch the space's speech between brief and full.
    Speech,
    /// Change the space's environment to this one, for everyone.
    Environment(String),
    /// Mute or unmute spoken replies here.
    Mute,
    /// Turn the status pill on or off here.
    Pill,
    /// Turn travel off here, or on again.
    Travel,
    /// Add a projection wall here, for everyone.
    AddWall,
    /// Remove the added wall nearest here, for everyone.
    RemoveWall,
    /// Leave the space.
    Leave,
    /// Delete the space, for everyone in it, and its agent session too when `session` is set.
    Delete {
        session: bool,
    },
    /// Step the sounds, or the vibration, to the next level.
    Sounds,
    Vibration,
    Mono,
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

/// A part of a row of the menu: text kept whole, such as "Turn ", " away" or a role, or the one part cut to fit, such as a name, mode or title.
enum Part<'a> {
    Kept(&'a str),
    Cut(&'a str),
}

/// `parts` as one row of at most [`WIDTH`] characters: the kept parts whole, and the part that may be cut taking the room they leave, ending with "…" where it needs more. A run of spaces or line breaks in it is one space.
fn row(parts: &[Part]) -> String {
    let kept: usize = parts
        .iter()
        .map(|part| match part {
            Part::Kept(text) => drawn(text).count(),
            Part::Cut(_) => 0,
        })
        .sum();
    let room = WIDTH.saturating_sub(kept);
    parts
        .iter()
        .map(|part| match part {
            Part::Kept(text) => (*text).to_owned(),
            Part::Cut(text) => clip(text, room),
        })
        .collect()
}

/// The rows of `section`, or of the top level, for `model`.
pub fn items(section: Option<Section>, model: &Model) -> Vec<Item> {
    // The Space and Agent sections apply only inside a space, so leaving one returns to the top level; this device's Sound section applies everywhere.
    let section = section.filter(|section| {
        model.in_space || matches!(section, Section::PeopleAndSecurity | Section::Sound)
    });
    let space = model.space.as_ref().filter(|_| model.in_space);
    // Only an owner is asked whether to delete the space, and whether to delete its session only where it can be.
    let deletion = space
        .and_then(|space| space.deletion)
        .filter(|_| model.role == ConnectionRole::Owner);
    let section = section.filter(|section| match section {
        Section::Delete { session } => deletion.is_some_and(|sessions| sessions || !session),
        Section::DeleteSession => deletion == Some(true),
        _ => true,
    });
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
        // The Space section goes on with the rest of the title, so only the top level marks it cut.
        let rows = if section.is_none() { 2 } else { usize::MAX };
        for line in lines(&space.title, WIDTH, rows).into_iter().take(2) {
            text(&mut items, MenuKind::Heading, line);
        }
        // The activity, a word, is kept, and the mode, a name, cut to leave room for the rest.
        let mut status = vec![Part::Kept(&space.activity)];
        if let Some(mode) = &space.mode {
            status.extend([Part::Kept(" | "), Part::Cut(mode)]);
        }
        if space.speech == tomori_protocol::SpeechSetting::Brief {
            status.push(Part::Kept(" | brief"));
        }
        status.extend([Part::Kept(" | "), Part::Kept(&space.here)]);
        text(&mut items, MenuKind::Text, row(&status));
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
            section_item(&mut items, Section::Sound, false, "Sound");
        }
        Some(Section::Sound) => {
            button(&mut items, MenuKey::Back, "< Back".into());
            text(&mut items, MenuKind::Heading, "On this headset".into());
            button(
                &mut items,
                MenuKey::Sounds,
                format!("Sounds: {}", model.feedback.sounds.label()),
            );
            button(
                &mut items,
                MenuKey::Vibration,
                format!("Vibration: {}", model.feedback.vibration.label()),
            );
            text(
                &mut items,
                MenuKind::Text,
                "Reduced: requests, failures, knocks".into(),
            );
            button(
                &mut items,
                MenuKey::Mono,
                format!(
                    "Mono audio: {}",
                    if model.feedback.mono { "on" } else { "off" }
                ),
            );
        }
        Some(Section::Space) => {
            button(&mut items, MenuKey::Back, "< Back".into());
            if let Some(space) = space {
                for line in lines(&space.title, WIDTH, usize::MAX).into_iter().skip(2) {
                    text(&mut items, MenuKind::Text, line);
                }
                text(
                    &mut items,
                    MenuKind::Text,
                    row(&[Part::Cut(&space.running)]),
                );
                text(&mut items, MenuKind::Heading, "Mode".into());
                if space.modes.is_empty() {
                    let mode = space.mode.as_deref().unwrap_or("none offered");
                    text(
                        &mut items,
                        MenuKind::Text,
                        row(&[Part::Kept("Mode: "), Part::Cut(mode)]),
                    );
                }
                for (id, name, _, current) in &space.modes {
                    let label = if *current {
                        row(&[Part::Kept("* "), Part::Cut(name), Part::Kept(" (now)")])
                    } else {
                        row(&[Part::Kept("  "), Part::Cut(name)])
                    };
                    // Only a participant switches it; a visitor reads it.
                    if model.role.participates() {
                        button(&mut items, MenuKey::Mode(id.clone()), label);
                    } else {
                        text(&mut items, MenuKind::Text, label);
                    }
                }
                // Beneath the modes, the description of the one pointed at, or else of the current one, in as many rows as the longest takes whichever is shown: the rows never depend on what is pointed at, so that pointing at a mode moves none of them, and the ray stays on the row it points at. See `design/hand-menu.md`.
                let described = |description: &Option<String>| {
                    lines(
                        description.as_deref().unwrap_or_default().trim(),
                        WIDTH - 2,
                        DESCRIPTION_ROWS,
                    )
                };
                let room = space
                    .modes
                    .iter()
                    .map(|(_, _, description, _)| described(description).len())
                    .max()
                    .unwrap_or(0);
                let pointed = |id: &String| model.pointed == Some(MenuKey::Mode(id.clone()));
                let mut description = space
                    .modes
                    .iter()
                    .find(|(id, ..)| pointed(id))
                    .or_else(|| space.modes.iter().find(|(.., current)| *current))
                    .map(|(_, _, description, _)| described(description))
                    .unwrap_or_default();
                description.resize(room, String::new());
                for line in description {
                    let line = if line.is_empty() {
                        line
                    } else {
                        format!("  {line}")
                    };
                    text(&mut items, MenuKind::Text, line);
                }
                if space.speech != tomori_protocol::SpeechSetting::Unknown {
                    text(&mut items, MenuKind::Heading, "Speech, for everyone".into());
                    let said = match space.speech {
                        tomori_protocol::SpeechSetting::Brief => "Brief: switch to full",
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
                if let Some(environment) = &space.environment {
                    text(
                        &mut items,
                        MenuKind::Heading,
                        "Environment, for everyone".into(),
                    );
                    // Only an owner changes it, to one of those the server offers; anyone else reads it.
                    if space.environments.is_empty() {
                        text(&mut items, MenuKind::Text, row(&[Part::Cut(environment)]));
                    }
                    for (id, name, current) in &space.environments {
                        if *current {
                            text(
                                &mut items,
                                MenuKind::Text,
                                row(&[Part::Kept("* "), Part::Cut(name), Part::Kept(" (now)")]),
                            );
                        } else {
                            button(
                                &mut items,
                                MenuKey::Environment(id.clone()),
                                row(&[Part::Kept("  "), Part::Cut(name)]),
                            );
                        }
                    }
                }
                // Only an owner adds and removes walls, for everyone.
                if let Some(removable) = space.walls {
                    text(
                        &mut items,
                        MenuKind::Heading,
                        "Projection walls, for everyone".into(),
                    );
                    button(&mut items, MenuKey::AddWall, "Add a wall here".into());
                    if removable {
                        button(
                            &mut items,
                            MenuKey::RemoveWall,
                            "Remove the nearest wall".into(),
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
                // Only where the environment allows travel.
                if let Some(travel) = model.travel {
                    button(
                        &mut items,
                        MenuKey::Travel,
                        if travel {
                            "Travel: on".into()
                        } else {
                            "Travel: off, room scale".into()
                        },
                    );
                }
                button(&mut items, MenuKey::Leave, "Leave space".into());
                // Apart from the rest, last, since it ends the space for everyone in it.
                if deletion.is_some() {
                    button(&mut items, MenuKey::Delete, "Delete this space".into());
                }
            }
        }
        // As many rows whether or not the session is to be deleted, so that turning it on moves none of them; the
        // choice that deletes is well above the last row, which keeps the space, where the ray was as the rows changed.
        Some(Section::Delete { session }) => {
            text(&mut items, MenuKind::Heading, "Delete this space?".into());
            let title = space.map_or("", |space| space.title.as_str());
            for line in lines(title, WIDTH, 2) {
                text(&mut items, MenuKind::Text, line);
            }
            button(
                &mut items,
                MenuKey::ConfirmDelete,
                "Delete it, for everyone".into(),
            );
            text(
                &mut items,
                MenuKind::Text,
                "Everyone here returns to the lobby.".into(),
            );
            text(
                &mut items,
                MenuKind::Text,
                if session {
                    "Its agent session is deleted too."
                } else {
                    "Its agent session is kept."
                }
                .into(),
            );
            if deletion == Some(true) {
                button(
                    &mut items,
                    MenuKey::DeleteSession,
                    if session {
                        "Also delete its agent session: yes".into()
                    } else {
                        "Also delete its agent session: no".into()
                    },
                );
            }
            button(&mut items, MenuKey::Back, "Keep this space".into());
        }
        Some(Section::DeleteSession) => {
            text(
                &mut items,
                MenuKind::Heading,
                "Delete its agent session too?".into(),
            );
            for line in lines(
                "Its conversation, and its workers' sessions, are lost for good. This cannot be undone.",
                WIDTH,
                NOTICE_ROWS,
            ) {
                text(&mut items, MenuKind::Text, line);
            }
            button(
                &mut items,
                MenuKey::ConfirmDeleteSession,
                "Delete the space and its session".into(),
            );
            text(&mut items, MenuKind::Text, String::new());
            button(
                &mut items,
                MenuKey::Back,
                "Back, keeping the session".into(),
            );
        }
        Some(Section::Leave) => {
            text(&mut items, MenuKind::Heading, "Leave this space?".into());
            let title = space.map_or("", |space| space.title.as_str());
            for line in lines(title, WIDTH, 2) {
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
                            // The start of every line of it, as one, cut at a word.
                            let room = WIDTH - layer.label().len() - 2;
                            let start = lines(&clip(said, usize::MAX), room, 1).concat();
                            button(
                                &mut items,
                                MenuKey::Layer(layer),
                                format!("{}: {start}", layer.label()),
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
                        for line in lines(error, WIDTH, LAYER_ROWS) {
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
            for line in lines(said, WIDTH, LAYER_ROWS) {
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
                        row(&[
                            Part::Kept("Let "),
                            Part::Cut(&knock.name),
                            Part::Kept(VISITOR),
                            Part::Kept(" in"),
                        ]),
                    );
                    button(
                        &mut items,
                        MenuKey::Decline(knock.id.clone()),
                        row(&[
                            Part::Kept("Turn "),
                            Part::Cut(&knock.name),
                            Part::Kept(VISITOR),
                            Part::Kept(" away"),
                        ]),
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
                button(&mut items, MenuKey::Pair, row(&[Part::Cut(pairing)]));
                requests = true;
            }
            if !requests {
                text(&mut items, MenuKind::Text, "Nothing waiting".into());
            }
            text(&mut items, MenuKind::Heading, "Roles & security".into());
            text(
                &mut items,
                MenuKind::Text,
                row(&[
                    Part::Kept("You: "),
                    Part::Cut(&model.name),
                    Part::Kept(" ("),
                    Part::Kept(role_label(model.role)),
                    Part::Kept(")"),
                ]),
            );
            // The name is said aloud, since typing on the headset is not practical yet; choosing again changes nothing after all.
            if model.renames {
                button(
                    &mut items,
                    MenuKey::Rename,
                    if model.naming {
                        "Hold X, say your name".into()
                    } else {
                        "Change your name".into()
                    },
                );
            }
            if model.in_space {
                if model.people.is_empty() {
                    let here = if model.participants.is_empty() {
                        "nobody".to_owned()
                    } else {
                        model.participants.join(", ")
                    };
                    // Wrapped, so that everyone here is shown.
                    for line in lines(&format!("Here: {here}"), WIDTH, usize::MAX) {
                        text(&mut items, MenuKind::Text, line);
                    }
                } else {
                    // Each with a swatch of their colour, the key to the transcript's colours.
                    text(&mut items, MenuKind::Text, "Here:".into());
                    for person in &model.people {
                        let (before, after) = if person.you { ("You (", ")") } else { ("", "") };
                        let away = if person.away { ", away" } else { "" };
                        let label = row(&[
                            Part::Kept(before),
                            Part::Cut(&person.name),
                            Part::Kept(after),
                            Part::Kept(away),
                        ]);
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
                for line in lines(notice, WIDTH, NOTICE_ROWS) {
                    text(&mut items, MenuKind::Text, line);
                }
            }
        }
    }
    // Every row is drawn as the transcript draws it, and fits the menu's width, which the rows above leave room for.
    for item in &mut items {
        item.text = plain(&cut(
            &[Span::new(item.text.as_str(), Style::plain([1.0; 4]))],
            WIDTH,
        ));
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

/// The hand menu's state: whether it shows, and which section is open.
#[derive(Clone, Debug, Default)]
pub struct HandMenu {
    palm_up: PalmUp,
    section: Option<Section>,
    /// Pinned open by Y, whatever the hand's pose, until Y or B.
    pinned: bool,
}

impl HandMenu {
    /// Update with the left palm at `now`, which shows the menu as the right palm shows the transcript, by [`PalmUp`]. Returns whether the menu is shown.
    pub fn update(&mut self, palm: Option<Palm>, now: Instant) -> bool {
        let shown = self.palm_up.update(palm, now);
        shown || self.pinned
    }

    /// Y in a space: pin the menu open at its Space section, whatever the hand's pose, or let go of it. See `design/hand-menu.md`.
    pub fn toggle_pin(&mut self) {
        self.pinned = !self.pinned;
        if self.pinned {
            self.section = Some(Section::Space);
        }
    }

    /// The space is to be deleted, with its session if `session`: the menu closes, as on leaving.
    fn deleted(&mut self, session: bool) -> Option<Choice> {
        self.section = None;
        self.pinned = false;
        Some(Choice::Delete { session })
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
                    Some(Section::Leave | Section::Delete { .. }) => Some(Section::Space),
                    // Back to deleting the space alone, or keeping it, which is asked again.
                    Some(Section::DeleteSession) => Some(Section::Delete { session: false }),
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
            // Asked afresh each time, with the session kept unless the owner turns its deletion on.
            MenuKey::Delete => {
                self.section = Some(Section::Delete { session: false });
                None
            }
            MenuKey::DeleteSession => {
                if let Some(Section::Delete { session }) = &mut self.section {
                    *session = !*session;
                }
                None
            }
            MenuKey::ConfirmDelete => match self.section {
                // Deleting the session as well is asked again, since it cannot be undone.
                Some(Section::Delete { session: true }) => {
                    self.section = Some(Section::DeleteSession);
                    None
                }
                Some(Section::Delete { session: false }) => self.deleted(false),
                _ => None,
            },
            MenuKey::ConfirmDeleteSession => match self.section {
                Some(Section::DeleteSession) => self.deleted(true),
                _ => None,
            },
            MenuKey::Mode(id) => Some(Choice::Mode(id.clone())),
            MenuKey::Speech => Some(Choice::Speech),
            MenuKey::Environment(id) => Some(Choice::Environment(id.clone())),
            MenuKey::Mute => Some(Choice::Mute),
            MenuKey::Pill => Some(Choice::Pill),
            MenuKey::Travel => Some(Choice::Travel),
            MenuKey::AddWall => Some(Choice::AddWall),
            MenuKey::RemoveWall => Some(Choice::RemoveWall),
            MenuKey::Layer(layer) => {
                self.section = Some(Section::Layer(*layer));
                None
            }
            MenuKey::ClearInstructions => Some(Choice::ClearInstructions),
            MenuKey::Sounds => Some(Choice::Sounds),
            MenuKey::Vibration => Some(Choice::Vibration),
            MenuKey::Mono => Some(Choice::Mono),
            MenuKey::Knock => Some(Choice::Knock),
            MenuKey::Admit(id) => Some(Choice::Admit(id.clone())),
            MenuKey::Decline(id) => Some(Choice::Decline(id.clone())),
            MenuKey::Pair => Some(Choice::Pair),
            MenuKey::Rename => Some(Choice::Rename),
            MenuKey::Text(_) | MenuKey::Cue | MenuKey::WristCue => None,
        }
    }

    /// The menu's rows for `model`, laid out above the wrist of `palm` on a plane facing `head`: each row is drawn within, and pointed at by, its [`row_rect`] in that plane.
    pub fn objects(&self, palm: &Palm, model: &Model, head: Vec3) -> Vec<Object> {
        let rows = items(self.section, model);
        let base = palm.wrist + palm.normal * 0.05;
        // The plane is aimed at the head from the menu's middle, so that the whole of it faces the participant; the middle depends on the aim, so it is refined once.
        let height = 0.04 + ROW * rows.len() as f32;
        let aim = |middle: Vec3| Plane::facing(base, facing_from(middle, Some(head), true));
        let first = aim(base + Vec3::Y * (height / 2.0));
        let plane = aim(base + first.up * (height / 2.0));
        // Its origin is the middle of the first row's marker.
        let plane = Plane {
            origin: base + plane.up * height,
            ..plane
        };
        rows.into_iter()
            .enumerate()
            .map(|(index, item)| {
                let middle = -ROW * index as f32;
                let position = plane.at(Vec2::new(0.0, middle));
                let choosable = matches!(item.kind, MenuKind::Section { .. } | MenuKind::Button);
                Object {
                    key: Key::Menu(item.key),
                    kind: Kind::Menu(item.kind),
                    position,
                    size: MARKER,
                    label: Label {
                        anchor: plane.at(Vec2::new(MARKER * 2.0, middle + 5.5 * PIXEL)),
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
                    panel: Some(Area {
                        plane,
                        rect: row_rect(index),
                    }),
                }
            })
            .collect()
    }
}

/// The rectangle of the menu's row `index` in its plane, whose origin is the middle of the first row's marker: from two markers' widths left of the middle of its marker to [`PANEL`] right of it, and a row tall, centred on its marker. The rows tile the panel, sharing their edges without gaps or overlaps. A row's marker and label, highlighted or not, are drawn within it, and it is what points at the row. See `design/hand-menu.md`.
pub fn row_rect(index: usize) -> Rect {
    // Each edge from one expression, so that neighbours share it exactly.
    let edge = |row: usize| ROW / 2.0 - ROW * row as f32;
    Rect {
        min: Vec2::new(-2.0 * MARKER, edge(index + 1)),
        max: Vec2::new(PANEL, edge(index)),
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
        panel: None,
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
    use tomori_protocol::Joint;

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

    /// A tracked hand on `side` with its palm up and its fingers forward: the index finger is on the thumb's side, left for a left hand and right for a right one.
    fn hand_palm_up(side: Side) -> Hand {
        let thumb = -palm_sign(side) * 0.03;
        hand(
            Vec3::ZERO,
            Vec3::new(thumb, 0.0, -0.09),
            Vec3::new(-thumb, 0.0, -0.08),
        )
    }

    /// A controller held in the hand on `side`, low in front, its -Z forward and its +Y up in a neutral grip, turned by `turn`.
    fn grip(side: Side, turn: Quat) -> controller::Pose {
        controller::Pose {
            position: [palm_sign(side) * -0.2, 1.1, -0.35],
            orientation: turn.to_array(),
        }
    }

    /// Rolling either hand outwards about the forward axis by 90° turns its palm up.
    fn rolled_up(side: Side) -> Quat {
        Quat::from_rotation_z(palm_sign(side) * std::f32::consts::FRAC_PI_2)
    }

    /// The palm of the controller on `side` turned up, then tilted by `tilt`.
    fn up_tilted(side: Side, tilt: Quat) -> Option<Palm> {
        Palm::from_controller(side, &grip(side, tilt * rolled_up(side)))
    }

    /// Whether `palm`, held for the dwell from hidden, shows.
    fn shows(palm: Option<Palm>) -> bool {
        let start = Instant::now();
        let mut palm_up = PalmUp::default();
        palm_up.update(palm, start);
        palm_up.update(palm, start + HOLD)
    }

    const SIDES: [Side; 2] = [Side::Left, Side::Right];

    #[test]
    fn either_palm_turned_up_faces_up_from_joints_or_the_grip() {
        for side in SIDES {
            let palm = Palm::from_hand(side, &hand_palm_up(side)).unwrap();
            assert!(
                palm.normal.dot(Vec3::Y) > 0.99,
                "{side:?} {:?}",
                palm.normal
            );
            let palm = up_tilted(side, Quat::IDENTITY).unwrap();
            assert!(
                palm.normal.dot(Vec3::Y) > 0.999,
                "{side:?} {:?}",
                palm.normal
            );
            // A hand missing a joint has no palm.
            let mut missing = hand_palm_up(side);
            missing.joints[WRIST] = None;
            assert_eq!(Palm::from_hand(side, &missing), None);
        }
        let cue = wrist_cue(&up_tilted(Side::Right, Quat::IDENTITY).unwrap(), true);
        assert_eq!(cue.kind, Kind::Menu(MenuKind::ProblemCue));
        assert_eq!(cue.key, Key::Menu(MenuKey::WristCue));
        assert!(!cue.choosable);
    }

    #[test]
    fn in_a_neutral_grip_either_palm_faces_inwards_along_opposite_signs_of_the_grip_x_axis() {
        // OpenXR's grip +X points away from the left palm and into the right one, so both palms face inwards, towards the body's middle, and 90° from up.
        let left = Palm::from_controller(Side::Left, &grip(Side::Left, Quat::IDENTITY)).unwrap();
        assert!(left.normal.dot(Vec3::X) > 0.999, "{:?}", left.normal);
        let right = Palm::from_controller(Side::Right, &grip(Side::Right, Quat::IDENTITY)).unwrap();
        assert!(right.normal.dot(-Vec3::X) > 0.999, "{:?}", right.normal);
        for palm in [left, right] {
            assert!((palm.angle_to_up().to_degrees() - 90.0).abs() < 0.01);
        }
        let degenerate = controller::Pose {
            position: [0.0; 3],
            orientation: [0.0; 4],
        };
        assert_eq!(Palm::from_controller(Side::Left, &degenerate), None);
    }

    #[test]
    fn either_palm_up_shows_after_the_dwell() {
        for side in SIDES {
            // A neutral grip, palm inwards, does not show, however long it is held.
            let neutral = Palm::from_controller(side, &grip(side, Quat::IDENTITY));
            assert!(!shows(neutral), "{side:?}");
            // Straight up shows, from joints or the grip.
            assert!(shows(up_tilted(side, Quat::IDENTITY)), "{side:?}");
            assert!(
                shows(Palm::from_hand(side, &hand_palm_up(side))),
                "{side:?}"
            );
            // So do 40° towards the body, away from it, forwards and back.
            for tilt in [
                Quat::from_rotation_z(40f32.to_radians()),
                Quat::from_rotation_z(-40f32.to_radians()),
                Quat::from_rotation_x(40f32.to_radians()),
                Quat::from_rotation_x(-40f32.to_radians()),
            ] {
                let palm = up_tilted(side, tilt);
                let angle = palm.unwrap().angle_to_up().to_degrees();
                assert!((angle - 40.0).abs() < 0.01, "{side:?} {angle}");
                assert!(shows(palm), "{side:?} {tilt:?}");
            }
            // Palm down does not.
            assert!(!shows(Palm::from_controller(
                side,
                &grip(side, rolled_up(side).inverse())
            )));
        }
    }

    #[test]
    fn either_palm_between_the_angles_neither_shows_nor_hides_and_past_them_hides() {
        let start = Instant::now();
        let tilt = |degrees: f32| Quat::from_rotation_z(degrees.to_radians());
        for side in SIDES {
            // 50° from up does not show from hidden.
            assert!(!shows(up_tilted(side, tilt(50.0))), "{side:?}");
            // But keeps it shown once it shows.
            let mut palm_up = PalmUp::default();
            palm_up.update(up_tilted(side, Quat::IDENTITY), start);
            assert!(palm_up.update(up_tilted(side, Quat::IDENTITY), start + HOLD));
            assert!(palm_up.update(up_tilted(side, tilt(50.0)), start + HOLD * 2));
            assert!(palm_up.update(up_tilted(side, tilt(-50.0)), start + HOLD * 3));
            // 70° hides it.
            assert!(!palm_up.update(up_tilted(side, tilt(70.0)), start + HOLD * 4));
        }
    }

    #[test]
    fn either_palm_respects_the_dwell_so_a_hand_passing_through_does_not_show() {
        let start = Instant::now();
        let up = |side| up_tilted(side, Quat::IDENTITY);
        let away = |side| up_tilted(side, Quat::from_rotation_z(60f32.to_radians()));
        for side in SIDES {
            let mut palm_up = PalmUp::default();
            assert!(!palm_up.update(up(side), start));
            assert!(!palm_up.update(up(side), start + HOLD / 2));
            assert!(palm_up.update(up(side), start + HOLD));
            assert!(!palm_up.update(away(side), start + HOLD * 2));
            // Passing through the pose, turning away before the dwell, restarts it.
            assert!(!palm_up.update(up(side), start + HOLD * 3));
            assert!(!palm_up.update(away(side), start + HOLD * 3 + HOLD / 2));
            assert!(!palm_up.update(up(side), start + HOLD * 4));
            assert!(!palm_up.update(up(side), start + HOLD * 4 + HOLD / 2));
            assert!(palm_up.update(up(side), start + HOLD * 5));
            // Losing the hand hides it.
            assert!(!palm_up.update(None, start + HOLD * 6));
        }
    }

    fn texts(items: &[Item]) -> Vec<&str> {
        items.iter().map(|item| item.text.as_str()).collect()
    }

    #[test]
    fn a_hand_or_controller_that_gives_no_direction_has_no_palm() {
        let flat = hand(Vec3::ZERO, Vec3::X, Vec3::X * 2.0);
        let lost = controller::Pose {
            position: [0.0; 3],
            orientation: [f32::NAN; 4],
        };
        for side in SIDES {
            assert_eq!(Palm::from_hand(side, &flat), None);
            for missing in [WRIST, INDEX_KNUCKLE, LITTLE_KNUCKLE] {
                let mut partial = hand(Vec3::ZERO, Vec3::X, Vec3::Z);
                partial.joints[missing] = None;
                assert_eq!(Palm::from_hand(side, &partial), None);
            }
            assert_eq!(Palm::from_controller(side, &lost), None);
        }
    }

    #[test]
    fn sections_say_what_they_hold_in_every_state() {
        let space = SpaceInfo {
            title:
                "A space whose title is long enough to wrap onto more lines than the header shows"
                    .into(),
            running: "fake in tmp".into(),
            speech: tomori_protocol::SpeechSetting::Full,
            ..Default::default()
        };
        let model = Model {
            in_space: true,
            role: ConnectionRole::Owner,
            pill: true,
            space: Some(space),
            feedback: tomori_client::feedback::Settings {
                mono: true,
                ..Default::default()
            },
            ..Default::default()
        };
        let shown = items(Some(Section::Space), &model);
        let shown = texts(&shown);
        assert!(shown.contains(&"Full: switch to brief"));
        assert!(shown.contains(&"Status pill: on"));
        let sound = items(Some(Section::Sound), &model);
        assert!(texts(&sound).contains(&"Mono audio: on"));
        // Without a space, the Space section offers only to go back; without instructions, the Agent section says none.
        let away = Model {
            in_space: true,
            ..Default::default()
        };
        assert_eq!(texts(&items(Some(Section::Space), &away))[0], "< Back");
        assert_eq!(texts(&items(Some(Section::Agent), &away))[0], "< Back");
        let short = Model {
            in_space: true,
            instructions: Some(tomori_protocol::AgentInstructions {
                space: "Be brief.".into(),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert!(texts(&items(Some(Section::Agent), &short)).contains(&"This space's: Be brief."));
        let layer = items(Some(Section::Layer(Layer::Space)), &short);
        assert_eq!(texts(&layer).last(), Some(&"Be brief."));
        for (state, said) in [
            (KnockState::Declined, "Your knock was declined"),
            (KnockState::Expired, "Nobody answered your knock"),
            (KnockState::Withdrawn, "Your knock was withdrawn"),
        ] {
            let knocked = Model {
                in_space: true,
                knock: Some(state),
                ..Default::default()
            };
            assert!(texts(&items(Some(Section::PeopleAndSecurity), &knocked)).contains(&said));
        }
    }

    #[test]
    fn the_top_level_flashes_people_and_security_while_someone_knocks() {
        let mut model = Model::default();
        let top = items(None, &model);
        assert_eq!(texts(&top), ["People & security", "Sound"]);
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
    fn the_sound_section_steps_the_feedback_setting_in_and_out_of_a_space() {
        use tomori_client::feedback::{Level, Settings};
        let mut model = Model {
            feedback: Settings {
                sounds: Level::Reduced,
                vibration: Level::Full,
                mono: false,
            },
            ..Default::default()
        };
        for in_space in [false, true] {
            model.in_space = in_space;
            assert_eq!(
                texts(&items(Some(Section::Sound), &model)),
                [
                    "< Back",
                    "On this headset",
                    "Sounds: reduced",
                    "Vibration: full",
                    "Reduced: requests, failures, knocks",
                    "Mono audio: off",
                ]
            );
        }
        let mut menu = HandMenu::default();
        assert_eq!(menu.choose(&MenuKey::Section(Section::Sound)), None);
        assert_eq!(menu.section, Some(Section::Sound));
        assert_eq!(menu.choose(&MenuKey::Sounds), Some(Choice::Sounds));
        assert_eq!(menu.choose(&MenuKey::Vibration), Some(Choice::Vibration));
        assert_eq!(menu.choose(&MenuKey::Mono), Some(Choice::Mono));
        assert_eq!(menu.choose(&MenuKey::Back), None);
        assert_eq!(menu.section, None);
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
                "Let Vic (visitor) in",
                "Turn Vic (visitor) away",
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
    fn a_participant_can_say_a_new_name_where_the_server_renames() {
        let mut model = Model {
            name: "Quest 3".into(),
            role: ConnectionRole::Collaborator,
            in_space: true,
            renames: true,
            ..Default::default()
        };
        let rows = items(Some(Section::PeopleAndSecurity), &model);
        let at = texts(&rows)
            .iter()
            .position(|text| *text == "You: Quest 3 (collaborator)")
            .unwrap();
        assert_eq!(rows[at + 1].key, MenuKey::Rename);
        assert_eq!(rows[at + 1].text, "Change your name");
        assert_eq!(rows[at + 1].kind, MenuKind::Button);
        // Chosen, the next press of X says it.
        model.naming = true;
        assert!(
            texts(&items(Some(Section::PeopleAndSecurity), &model))
                .contains(&"Hold X, say your name")
        );
        let mut menu = HandMenu::default();
        assert_eq!(menu.choose(&MenuKey::Rename), Some(Choice::Rename));
        // A server that does not rename offers nothing.
        model.renames = false;
        assert!(
            !items(Some(Section::PeopleAndSecurity), &model)
                .iter()
                .any(|item| item.key == MenuKey::Rename)
        );
    }

    #[test]
    fn the_agent_section_shows_each_layer_in_a_space_and_opens_it() {
        let mut model = Model {
            role: ConnectionRole::Owner,
            ..Default::default()
        };
        // Only inside a space, where it applies.
        assert_eq!(texts(&items(None, &model)), ["People & security", "Sound"]);
        assert_eq!(
            texts(&items(Some(Section::Agent), &model)),
            ["People & security", "Sound"]
        );
        model.in_space = true;
        assert_eq!(
            texts(&items(None, &model)),
            ["Space", "Agent", "People & security", "Sound"]
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
            space: "Mind the docs.\nAnd more".into(),
            error: Some("The agent did not take Tomori's instructions: gone".into()),
        });
        let rows = items(Some(Section::Agent), &model);
        assert_eq!(
            texts(&rows),
            [
                "< Back",
                "Instructions",
                "Tomori's: You are Tomori, the…",
                "The environment's: none",
                "This space's: Mind the docs. And more",
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
        assert!(rows.last().unwrap().text.ends_with('…'));
        assert!(rows.iter().all(|row| row.text.chars().count() <= WIDTH));
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
                speech: tomori_protocol::SpeechSetting::Brief,
                environment: Some("Sky".into()),
                environments: Vec::new(),
                here: "2 here".into(),
                walls: None,
                deletion: None,
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
                "Fix the parser in the lexer before the",
                "release goes out",
                "running | build | brief | 2 here",
                "Space",
                "Agent",
                "People & security",
                "Sound",
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
            ["People & security", "Sound"]
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
                "  Makes changes.",
                "Speech, for everyone",
                "Brief: switch to full",
                "Environment, for everyone",
                "Sky",
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
        // Beneath the modes is the description of the current one, or of the one pointed at, in the same row.
        model.pointed = Some(MenuKey::Mode("plan".into()));
        let shown = texts(&items(Some(Section::Space), &model))
            .iter()
            .map(|text| text.to_string())
            .collect::<Vec<_>>();
        let plan = shown.iter().position(|text| text == "  plan").unwrap();
        assert_eq!(shown[plan + 1], "  Plans without changing files.");
        assert_eq!(shown.len(), rows.len());
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
        // Where the environment allows travel, this headset can turn it off, beneath the pill.
        assert!(!rows.iter().any(|row| row.key == MenuKey::Travel));
        for (travel, said) in [(true, "Travel: on"), (false, "Travel: off, room scale")] {
            model.travel = Some(travel);
            let rows = items(Some(Section::Space), &model);
            let pill = rows
                .iter()
                .position(|row| row.key == MenuKey::Pill)
                .unwrap();
            assert_eq!(rows[pill + 1].key, MenuKey::Travel);
            assert_eq!(rows[pill + 1].text, said);
            assert_eq!(rows[pill + 2].key, MenuKey::Leave);
        }
        assert_eq!(menu.choose(&MenuKey::Travel), Some(Choice::Travel));
        // An owner adds walls, and removes the nearest added one where there is one, before this headset's settings.
        assert!(!rows.iter().any(|row| row.key == MenuKey::AddWall));
        for (removable, keys) in [
            (false, vec![MenuKey::AddWall]),
            (true, vec![MenuKey::AddWall, MenuKey::RemoveWall]),
        ] {
            model.space.as_mut().unwrap().walls = Some(removable);
            let rows = items(Some(Section::Space), &model);
            let heading = rows
                .iter()
                .position(|row| row.text == "Projection walls, for everyone")
                .unwrap();
            let shown: Vec<MenuKey> = rows[heading + 1..=heading + keys.len()]
                .iter()
                .map(|row| row.key.clone())
                .collect();
            assert_eq!(shown, keys);
            assert_eq!(rows[heading + keys.len() + 1].text, "On this headset");
        }
        assert_eq!(menu.choose(&MenuKey::AddWall), Some(Choice::AddWall));
        assert_eq!(menu.choose(&MenuKey::RemoveWall), Some(Choice::RemoveWall));
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
    fn only_an_owner_is_offered_the_space_s_environments() {
        let mut owner = in_space(ConnectionRole::Owner);
        // A server that changes environments offers its environments to an owner, the current one marked.
        let space = owner.space.as_mut().unwrap();
        space.environments = vec![
            ("sky".into(), "Sky".into(), true),
            ("minimal".into(), "Grid".into(), false),
        ];
        let rows = items(Some(Section::Space), &owner);
        let shown = texts(&rows);
        let heading = shown
            .iter()
            .position(|text| *text == "Environment, for everyone")
            .unwrap();
        assert_eq!(
            shown[heading..heading + 4],
            [
                "Environment, for everyone",
                "* Sky (now)",
                "  Grid",
                "On this headset"
            ]
        );
        // Only the others can be chosen.
        assert_eq!(rows[heading + 1].kind, MenuKind::Text);
        assert_eq!(
            rows[heading + 2].key,
            MenuKey::Environment("minimal".into())
        );
        let mut menu = HandMenu::default();
        assert_eq!(
            menu.choose(&MenuKey::Environment("minimal".into())),
            Some(Choice::Environment("minimal".into()))
        );
        // Anyone else reads the environment and chooses none.
        let collaborator = in_space(ConnectionRole::Collaborator);
        let rows = items(Some(Section::Space), &collaborator);
        assert!(texts(&rows).contains(&"Sky"));
        assert!(
            !rows
                .iter()
                .any(|row| matches!(row.key, MenuKey::Environment(_)))
        );
        // An older server names no environment, so none is shown.
        let mut older = in_space(ConnectionRole::Owner);
        older.space.as_mut().unwrap().environment = None;
        assert!(
            !texts(&items(Some(Section::Space), &older)).contains(&"Environment, for everyone")
        );
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
                "Fix the parser in the lexer before the",
                "release goes out",
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

    /// An owner in a space on a server that deletes spaces, and its sessions when `sessions`.
    fn deletable(sessions: bool) -> Model {
        let mut model = in_space(ConnectionRole::Owner);
        model.space.as_mut().unwrap().deletion = Some(sessions);
        model
    }

    #[test]
    fn deleting_from_the_menu_asks_first_and_again_for_the_session() {
        let model = deletable(true);
        let mut menu = HandMenu::default();
        menu.toggle_pin();
        let space = items(menu.section, &model);
        // Last, apart from the rest of the space's rows.
        assert_eq!(space.last().unwrap().key, MenuKey::Delete);
        assert_eq!(space.last().unwrap().text, "Delete this space");
        assert_eq!(menu.choose(&MenuKey::Delete), None);
        assert_eq!(menu.section, Some(Section::Delete { session: false }));
        let kept = items(menu.section, &model);
        assert_eq!(
            texts(&kept),
            [
                "Delete this space?",
                "Fix the parser in the lexer before the",
                "release goes out",
                "Delete it, for everyone",
                "Everyone here returns to the lobby.",
                "Its agent session is kept.",
                "Also delete its agent session: no",
                "Keep this space",
            ]
        );
        // The last row, where the ray was as the rows changed, keeps the space, and the one that deletes is well above it.
        assert_eq!(kept.last().unwrap().key, MenuKey::Back);
        assert_eq!(kept[3].key, MenuKey::ConfirmDelete);
        // Turning the session's deletion on says so, and moves no row.
        assert_eq!(menu.choose(&MenuKey::DeleteSession), None);
        assert_eq!(menu.section, Some(Section::Delete { session: true }));
        let both = items(menu.section, &model);
        assert_eq!(both.len(), kept.len());
        assert_eq!(both[5].text, "Its agent session is deleted too.");
        assert_eq!(both[6].text, "Also delete its agent session: yes");
        menu.choose(&MenuKey::DeleteSession);
        assert_eq!(menu.section, Some(Section::Delete { session: false }));
        // Keeping it steps back to the Space section, and asking again starts with the session kept.
        menu.choose(&MenuKey::DeleteSession);
        assert_eq!(menu.choose(&MenuKey::Back), None);
        assert_eq!(menu.section, Some(Section::Space));
        menu.choose(&MenuKey::Delete);
        assert_eq!(menu.section, Some(Section::Delete { session: false }));

        // With the session, deleting asks again, and stepping back from that keeps the session.
        menu.choose(&MenuKey::DeleteSession);
        assert_eq!(menu.choose(&MenuKey::ConfirmDelete), None);
        assert_eq!(menu.section, Some(Section::DeleteSession));
        let again = items(menu.section, &model);
        assert_eq!(
            texts(&again),
            [
                "Delete its agent session too?",
                "Its conversation, and its workers'",
                "sessions, are lost for good. This cannot",
                "be undone.",
                "Delete the space and its session",
                "",
                "Back, keeping the session",
            ]
        );
        assert_eq!(again.last().unwrap().key, MenuKey::Back);
        assert_eq!(menu.choose(&MenuKey::Back), None);
        assert_eq!(menu.section, Some(Section::Delete { session: false }));
        menu.choose(&MenuKey::DeleteSession);
        menu.choose(&MenuKey::ConfirmDelete);
        assert_eq!(
            menu.choose(&MenuKey::ConfirmDeleteSession),
            Some(Choice::Delete { session: true })
        );
        // The menu closes, and lets go, as on leaving.
        assert_eq!(menu.section, None);
        assert!(!menu.pinned());

        // Without the session, deleting needs no second asking.
        menu.choose(&MenuKey::Section(Section::Space));
        menu.choose(&MenuKey::Delete);
        assert_eq!(
            menu.choose(&MenuKey::ConfirmDelete),
            Some(Choice::Delete { session: false })
        );
        assert_eq!(menu.section, None);

        // A choice left from rows no longer shown deletes nothing.
        for section in [
            None,
            Some(Section::Space),
            Some(Section::Delete { session: false }),
        ] {
            menu.section = section;
            assert_eq!(menu.choose(&MenuKey::ConfirmDeleteSession), None);
            assert_eq!(menu.section, section);
        }
        menu.section = Some(Section::Space);
        assert_eq!(menu.choose(&MenuKey::ConfirmDelete), None);
        assert_eq!(menu.choose(&MenuKey::DeleteSession), None);
        assert_eq!(menu.section, Some(Section::Space));
    }

    #[test]
    fn only_owners_are_asked_whether_to_delete_a_space() {
        let sections = [
            Some(Section::Delete { session: false }),
            Some(Section::Delete { session: true }),
            Some(Section::DeleteSession),
        ];
        // Anyone else, or an owner on an older server, is offered nothing, and the rows that ask show the top level instead.
        for model in [
            in_space(ConnectionRole::Owner),
            {
                let mut model = deletable(true);
                model.role = ConnectionRole::Collaborator;
                model
            },
            {
                let mut model = deletable(true);
                model.role = ConnectionRole::Visitor;
                model
            },
            {
                let mut model = deletable(true);
                model.role = ConnectionRole::Unknown;
                model
            },
        ] {
            assert!(
                !items(Some(Section::Space), &model)
                    .iter()
                    .any(|row| row.key == MenuKey::Delete),
                "{:?}",
                model.role
            );
            for section in sections {
                assert_eq!(items(section, &model), items(None, &model));
            }
        }
        // Where the session cannot be deleted, the space can, and the session is kept without asking.
        let model = deletable(false);
        let rows = items(Some(Section::Delete { session: false }), &model);
        assert!(!rows.iter().any(|row| row.key == MenuKey::DeleteSession));
        assert!(texts(&rows).contains(&"Its agent session is kept."));
        assert_eq!(rows.last().unwrap().key, MenuKey::Back);
        for section in &sections[1..] {
            assert_eq!(items(*section, &model), items(None, &model));
        }
        // Outside a space, there is nothing to delete.
        let mut lobby = deletable(true);
        lobby.in_space = false;
        assert_eq!(
            items(Some(Section::Delete { session: false }), &lobby),
            items(None, &lobby)
        );
    }

    #[test]
    fn y_pins_the_menu_open_at_the_space_section_until_y_or_b() {
        let start = Instant::now();
        let mut menu = HandMenu::default();
        assert!(!menu.update(None, start));
        menu.toggle_pin();
        assert!(menu.pinned());
        assert_eq!(menu.section, Some(Section::Space));
        // Shown whatever the hand's pose, or with no hand at all.
        assert!(menu.update(None, start));
        menu.toggle_pin();
        assert!(!menu.update(None, start));
        menu.toggle_pin();
        assert!(menu.unpin());
        assert!(!menu.unpin());
        assert!(!menu.update(None, start));
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
        // Every row is a rectangle of one panel, whose plane holds its marker and its label's anchor.
        let plane = objects[0].panel.unwrap().plane;
        for (index, object) in objects.iter().enumerate() {
            let area = object.panel.unwrap();
            assert_eq!(area.plane, plane);
            assert_eq!(area.rect, row_rect(index));
            for point in [object.position, object.label.anchor] {
                assert!((point - plane.origin).dot(plane.normal()).abs() < 1e-5);
                assert!(area.rect.contains(area.local(point)), "{index}");
            }
        }
    }

    /// The rows tile the panel: each a row tall, from left of its marker to [`PANEL`], sharing its edges with its neighbours.
    #[test]
    fn the_rows_rectangles_tile_the_panel() {
        for index in 0..20 {
            let rect = row_rect(index);
            assert!((rect.max.y - rect.min.y - ROW).abs() < 1e-6);
            assert_eq!(rect.max.x, PANEL);
            assert!(rect.contains(Vec2::new(0.0, -ROW * index as f32)));
            assert_eq!(rect.min.y, row_rect(index + 1).max.y);
        }
    }

    /// An owner's Space section, whose buttons and rows that are only read alternate, with a mode whose name fills its row, and no mode described.
    fn alternating() -> (HandMenu, Model) {
        let mut model = in_space(ConnectionRole::Owner);
        let space = model.space.as_mut().unwrap();
        for mode in &mut space.modes {
            mode.2 = None;
        }
        space
            .modes
            .push(("long".into(), "x".repeat(2 * WIDTH), None, false));
        let menu = HandMenu {
            section: Some(Section::Space),
            ..Default::default()
        };
        (menu, model)
    }

    /// Pointing at any row of any section, a mode most of all, moves no row: what is pointed at changes what the rows say, never where they are, so that the ray stays on the row it points at. The menu grew by the description of a mode pointed at, which raised every row by as many, and the next frame the ray met another.
    #[test]
    fn pointing_at_a_row_moves_no_row() {
        let (_, mut model) = alternating();
        let space = model.space.as_mut().unwrap();
        space.modes[0].2 = Some("Makes changes.".into());
        space.modes[1].2 = Some("Plans without changing files. ".repeat(4));
        space.environments = vec![
            ("sky".into(), "Sky".into(), true),
            ("minimal".into(), "Grid".into(), false),
        ];
        let palm = Palm {
            wrist: Vec3::new(-0.15, 1.05, -0.35),
            normal: Vec3::Y,
        };
        let head = Vec3::new(0.0, 1.6, 0.0);
        let layout = |menu: &HandMenu, model: &Model| {
            menu.objects(&palm, model, head)
                .into_iter()
                .map(|object| (object.key, object.position, object.panel))
                .collect::<Vec<_>>()
        };
        for section in [None, Some(Section::Space), Some(Section::Sound)] {
            let menu = HandMenu {
                section,
                ..Default::default()
            };
            model.pointed = None;
            let still = layout(&menu, &model);
            for item in items(section, &model) {
                let key = item.key;
                model.pointed = Some(key.clone());
                assert_eq!(layout(&menu, &model), still, "{section:?}: {key:?}");
            }
        }
        // The description shown is the one pointed at, and otherwise the current mode's.
        model.pointed = Some(MenuKey::Mode("plan".into()));
        assert!(
            rows(Some(Section::Space), &model)
                .contains(&"  Plans without changing files. Plans".to_owned())
        );
        model.pointed = Some(MenuKey::Mode("long".into()));
        let long = rows(Some(Section::Space), &model);
        assert!(
            !long
                .iter()
                .any(|row| row.contains("Makes") || row.contains("Plans")),
            "{long:?}"
        );
        model.pointed = Some(MenuKey::Speech);
        assert!(rows(Some(Section::Space), &model).contains(&"  Makes changes.".to_owned()));
        // With no modes there is no description.
        model.space.as_mut().unwrap().modes.clear();
        assert!(
            !rows(Some(Section::Space), &model)
                .iter()
                .any(|row| row.is_empty())
        );
    }

    /// The menu of [`alternating`] held palm up, at chest height, with the head where its panel tilts `tilt` degrees back to face it.
    fn tilted_back(tilt: f32) -> Vec<Object> {
        let (menu, model) = alternating();
        let palm = Palm {
            wrist: Vec3::new(-0.15, 1.05, -0.35),
            normal: Vec3::Y,
        };
        // The panel faces the head from its middle, which is half its height up the tilted panel from above the wrist.
        let half = (0.04 + ROW * items(menu.section, &model).len() as f32) / 2.0;
        let (sin, cos) = tilt.to_radians().sin_cos();
        let (normal, up) = (Vec3::new(0.0, sin, cos), Vec3::new(0.0, cos, -sin));
        let middle = palm.wrist + palm.normal * 0.05 + up * half;
        let objects = menu.objects(&palm, &model, middle + normal * 0.45);
        let held = objects[0].panel.unwrap().plane.normal();
        let measured = held.y.asin().to_degrees();
        assert!((measured - tilt).abs() < 2.0, "{tilt}: {measured}");
        objects
    }

    /// Rays from the head, and from a right hand straight ahead of the panel, below it, above it, beside it, and steeply up from beneath it, at the point `at` of `plane`.
    fn rays_at(plane: &Plane, head: Vec3, at: Vec2) -> Vec<crate::pointing::Ray> {
        let point = plane.at(at);
        let (right, up, normal) = (plane.right, plane.up, plane.normal());
        [
            head,
            point + normal * 0.5,
            point + normal * 0.35 + right * 0.25 - up * 0.3,
            point + normal * 0.3 + up * 0.25,
            point + normal * 0.2 - right * 0.3,
            point + normal * 0.12 - up * 0.45,
        ]
        .into_iter()
        .map(|origin| crate::pointing::Ray::new(origin, point - origin).unwrap())
        .collect()
    }

    /// A ray aimed anywhere at a row meets that row, and none of its neighbours, whichever way the menu tilts and from wherever it is aimed: across the whole of a long row to the panel's right edge, and on either side of the edges between rows. A row that is only read is met by nothing.
    #[test]
    fn a_ray_aimed_at_a_row_meets_that_row_at_any_tilt() {
        for tilt in [0.0, 30.0, 60.0] {
            let objects = tilted_back(tilt);
            let scene = crate::arrival::Scene {
                objects: objects.clone(),
                ..Default::default()
            };
            let targets = scene.targets();
            let plane = objects[0].panel.unwrap().plane;
            let (sin, cos) = f32::to_radians(tilt).sin_cos();
            let head = plane.at(Vec2::new(0.0, -0.2)) + Vec3::new(0.0, sin, cos) * 0.45;
            // The row of the long mode reaches the panel's right edge.
            let long = objects
                .iter()
                .position(|object| object.key == Key::Menu(MenuKey::Mode("long".into())))
                .unwrap();
            assert_eq!(objects[long].label.lines[0].chars().count(), WIDTH);
            let mut met = 0;
            for (index, object) in objects.iter().enumerate() {
                let rect = row_rect(index);
                let (top, bottom) = (rect.max.y, rect.min.y);
                let middle = (top + bottom) / 2.0;
                let inside = [
                    Vec2::new(0.0, middle),
                    rect.centre(),
                    Vec2::new(rect.min.x + 0.001, top - 0.0002),
                    Vec2::new(PANEL - 0.001, bottom + 0.0002),
                    Vec2::new(PANEL - 0.001, middle),
                ];
                let expected = object.choosable.then(|| object.key.clone());
                for at in inside {
                    for ray in rays_at(&plane, head, at) {
                        let hit = crate::pointing::nearest(&ray, &targets).map(|hit| hit.key);
                        assert_eq!(hit, expected, "{tilt}° row {index} at {at} from {ray:?}");
                        met += usize::from(hit.is_some());
                    }
                }
                // Past either end of the row, nothing is met.
                for at in [
                    Vec2::new(PANEL + 0.002, middle),
                    Vec2::new(rect.min.x - 0.002, middle),
                ] {
                    for ray in rays_at(&plane, head, at) {
                        assert_eq!(crate::pointing::nearest(&ray, &targets), None, "{at}");
                    }
                }
            }
            assert!(met > 100, "{met}");
        }
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
    fn showing_follows_the_palm() {
        let start = Instant::now();
        let mut menu = HandMenu::default();
        let palm = Palm {
            wrist: Vec3::new(0.0, 1.0, -0.3),
            normal: Vec3::new(0.0, 0.6, 0.3).normalize(),
        };
        assert!(!menu.update(Some(palm), start));
        assert!(menu.update(Some(palm), start + HOLD));
        assert!(!menu.update(None, start + HOLD * 2));
        for role in [
            ConnectionRole::Collaborator,
            ConnectionRole::Pairing,
            ConnectionRole::Unknown,
        ] {
            assert!(!role_label(role).is_empty());
        }
    }
    #[test]
    fn every_row_fits_the_menu_and_draws_what_the_transcript_draws() {
        let long = "Bartholomew-Fitzgerald-Montgomery-Smythe";
        let mut model = in_space(ConnectionRole::Owner);
        model.name = format!("José {long}");
        model.participants = vec![long.into(); 4];
        model.people.clear();
        model.pairing = Some(format!("Pairing code {}", "9".repeat(60)));
        model.knocks = vec![KnockRequest {
            id: "knock-1".into(),
            name: long.repeat(2),
        }];
        model.notice = Some("A security notice long enough to be cut. ".repeat(4));
        let space = model.space.as_mut().unwrap();
        space.deletion = Some(true);
        space.title = format!("{} {}", "word".repeat(15), "Łódź ".repeat(20));
        space.activity = "running a very long tool call".into();
        space.here = "12 here, 3 away".into();
        space.modes[1].1 = long.repeat(3);
        space.modes[1].2 = Some("A description ".repeat(30));
        space.environments = vec![
            ("sky".into(), "Sky".into(), true),
            ("long".into(), long.repeat(2), false),
        ];
        model.pointed = Some(MenuKey::Mode("plan".into()));
        model.instructions = Some(AgentInstructions {
            tomori: "x".repeat(400),
            environment: "Short.".into(),
            space: "word ".repeat(400),
            error: Some("why ".repeat(200)),
        });
        let sections = [
            None,
            Some(Section::Space),
            Some(Section::Leave),
            Some(Section::Delete { session: false }),
            Some(Section::Delete { session: true }),
            Some(Section::DeleteSession),
            Some(Section::Agent),
            Some(Section::Layer(Layer::Tomori)),
            Some(Section::Layer(Layer::Space)),
            Some(Section::PeopleAndSecurity),
            Some(Section::Sound),
        ];
        // Japanese takes two cells a character, and every row still fits, counted in cells.
        let mut japanese = model.clone();
        japanese.name = "山田太郎".repeat(5);
        japanese.participants = vec!["佐藤花子".repeat(6); 2];
        let space = japanese.space.as_mut().unwrap();
        space.title = "日本語の会話のスペース。".repeat(8);
        space.modes[1].2 = Some("説明文です。".repeat(30));
        // And so does Latin mixed with Japanese, and emoji, which no font has, drawn `?` in one cell.
        let mut mixed = model.clone();
        mixed.name = "Bob 佐藤 🎤 José".repeat(4);
        mixed.participants = vec!["Ada 🎧 山田".repeat(5); 3];
        let space = mixed.space.as_mut().unwrap();
        space.title = "Tomoriは日本語 🎉 mixed text".repeat(6);
        space.modes[1].2 = Some("Fix the パーサー bug 🐛 ".repeat(20));
        for model in [&model, &japanese, &mixed] {
            for section in sections {
                let rows = items(section, model);
                for row in &rows {
                    assert!(
                        crate::ui_text::width(&row.text) <= WIDTH,
                        "{section:?}: {}",
                        row.text
                    );
                    let cells: Vec<_> = drawn(&row.text).map(|ch| (ch, ())).collect();
                    assert!(crate::ui_text::whole(&cells), "{section:?}: {}", row.text);
                }
            }
        }
        assert!(
            texts(&items(Some(Section::Space), &japanese))
                .iter()
                .any(|row| row.contains("日本語の会話"))
        );
        assert!(
            texts(&items(Some(Section::Space), &mixed))
                .iter()
                .any(|row| row.contains("Tomoriは日本語 ? mixed"))
        );
        // The top level marks a title cut at two rows; the Space section goes on with the rest of it.
        let top = items(None, &model);
        assert_eq!(top[1].text.chars().last(), Some('…'));
        assert!(!top[0].text.contains('?'));
        let space = texts(&items(Some(Section::Space), &model))
            .iter()
            .map(|text| text.to_string())
            .collect::<Vec<_>>();
        assert!(!space[1].ends_with('…'));
        assert!(space.iter().any(|row| row.contains("Łódź")), "{space:?}");
        // A word longer than a row is broken across rows rather than dropped.
        let layer = texts(&items(Some(Section::Layer(Layer::Tomori)), &model))
            .iter()
            .map(|text| text.to_string())
            .collect::<Vec<_>>();
        assert_eq!(layer[2], "x".repeat(WIDTH));
        assert_eq!(layer.len(), 2 + 10);
        // The description of the mode pointed at, the error and the notice are bounded in rows too, and marked where cut.
        let people = texts(&items(Some(Section::PeopleAndSecurity), &model))
            .iter()
            .map(|text| text.to_string())
            .collect::<Vec<_>>();
        assert!(people.last().unwrap().ends_with('…'));
        let pointed = items(Some(Section::Space), &model);
        assert!(pointed.iter().any(|row| row.text.ends_with('…')));
        // Its rows are kept whatever is pointed at, empty beneath the current mode's shorter description.
        model.pointed = None;
        let current = items(Some(Section::Space), &model);
        assert_eq!(current.len(), pointed.len());
        let after = |rows: &[Item]| {
            let last = rows
                .iter()
                .rposition(|row| matches!(row.key, MenuKey::Mode(_)))
                .unwrap();
            rows[last + 1..last + 1 + DESCRIPTION_ROWS]
                .iter()
                .map(|row| row.text.clone())
                .collect::<Vec<_>>()
        };
        assert!(
            after(&pointed)
                .iter()
                .all(|row| row.starts_with("  ") && row.contains("description"))
        );
        assert_eq!(after(&current), ["  Makes changes.", "", "", ""]);
    }

    /// The rows of `section` for `model`, as text.
    fn rows(section: Option<Section>, model: &Model) -> Vec<String> {
        texts(&items(section, model))
            .into_iter()
            .map(str::to_owned)
            .collect()
    }

    /// Whether `rows` has one that starts with `start`, cuts what follows with "…" and ends with `end`, filling the menu's width.
    fn cut_between(rows: &[String], start: &str, end: &str) -> bool {
        rows.iter().any(|row| {
            row.starts_with(start)
                && row.ends_with(&format!("…{end}"))
                && row.chars().count() == WIDTH
        })
    }

    /// The text of the row of `section` chosen by `key`.
    fn text_of(section: Option<Section>, model: &Model, key: &MenuKey) -> String {
        items(section, model)
            .into_iter()
            .find(|item| &item.key == key)
            .expect("a row with the key")
            .text
    }

    #[test]
    fn a_long_name_title_or_label_is_cut_and_the_rest_of_its_row_kept() {
        // Names, modes and environments of up to 64 bytes are allowed, and spoken.
        let long = "Bartholomew Fitzgerald-Montgomery of the Very Long Name";
        let mut model = in_space(ConnectionRole::Owner);
        model.name = long.into();
        model.knocks = vec![KnockRequest {
            id: "knock-1".into(),
            name: long.into(),
        }];
        model.people[0].name = long.into();
        model.people[1].name = long.into();
        model.people.push(Person {
            name: long.into(),
            colour: [1.0; 3],
            you: false,
            away: false,
        });
        model.pairing = Some(long.into());
        // The role, "away", "in" and the action words survive.
        let people = rows(Some(Section::PeopleAndSecurity), &model);
        // A knock's visitor stays marked as one, however long their name.
        assert!(
            cut_between(&people, "Let Bartholomew", " (visitor) in"),
            "{people:?}"
        );
        assert!(
            cut_between(&people, "Turn Bartholomew", " (visitor) away"),
            "{people:?}"
        );
        assert!(
            cut_between(&people, "You: Bartholomew", " (owner)"),
            "{people:?}"
        );
        assert!(cut_between(&people, "You (Bartholomew", ")"), "{people:?}");
        assert!(cut_between(&people, "Bartholomew", ", away"), "{people:?}");
        // A row that is only a name, such as a pairing label or someone here, is cut to the width.
        let whole = clip(long, WIDTH);
        assert_eq!(whole.chars().count(), WIDTH);
        assert_eq!(
            text_of(Some(Section::PeopleAndSecurity), &model, &MenuKey::Pair),
            whole
        );
        assert_eq!(people.iter().filter(|row| **row == whole).count(), 2);
        // A visitor's own row keeps its role too.
        model.role = ConnectionRole::Visitor;
        let visitor = rows(Some(Section::PeopleAndSecurity), &model);
        assert!(
            cut_between(&visitor, "You: Bartholomew", " (visitor)"),
            "{visitor:?}"
        );

        // In the Space section, a long mode keeps "(now)", and the title, environment and status are cut or wrapped to fit, keeping the rest.
        let mut model = in_space(ConnectionRole::Owner);
        let space = model.space.as_mut().unwrap();
        space.title = format!("{long} {long}");
        space.running = format!("opencode in {long}");
        space.mode = Some(long.into());
        space.modes[0].1 = long.into();
        space.modes[1].1 = long.into();
        space.here = "12 here, 3 away".into();
        space.environment = Some(long.into());
        space.environments = vec![
            ("current".into(), long.into(), true),
            ("other".into(), long.into(), false),
        ];
        let rows_of_space = rows(Some(Section::Space), &model);
        let now = format!("* {} (now)", clip(long, WIDTH - 8));
        let other = format!("  {}", clip(long, WIDTH - 2));
        assert!(
            cut_between(std::slice::from_ref(&now), "* Bartholomew", " (now)"),
            "{now}"
        );
        assert!(
            cut_between(std::slice::from_ref(&other), "  Bartholomew", ""),
            "{other}"
        );
        let space_row = |key: MenuKey| text_of(Some(Section::Space), &model, &key);
        assert_eq!(space_row(MenuKey::Mode("build".into())), now);
        assert_eq!(space_row(MenuKey::Mode("plan".into())), other);
        assert_eq!(space_row(MenuKey::Environment("other".into())), other);
        // The current environment is read, not chosen, after the mode's rows.
        assert_eq!(rows_of_space.iter().filter(|row| **row == now).count(), 2);
        assert!(
            rows_of_space
                .iter()
                .any(|row| row.starts_with("opencode in Bartholomew") && row.ends_with('…')),
            "{rows_of_space:?}"
        );
        // The status keeps the activity, "brief" and how many are here, and cuts the mode.
        assert_eq!(rows_of_space[2], "running | Bar… | brief | 12 here, 3 away");
        model.space.as_mut().unwrap().speech = tomori_protocol::SpeechSetting::Full;
        assert!(cut_between(
            &rows(Some(Section::Space), &model),
            "running | Bartholomew",
            " | 12 here, 3 away"
        ));
        // The title wraps at words, on the top level's two rows and the Space section's after them, before the space's running agent.
        let heading = items(Some(Section::Space), &model);
        assert_eq!(heading[0].kind, MenuKind::Heading);
        assert_eq!(heading[3].key, MenuKey::Back);
        let running = rows_of_space
            .iter()
            .position(|row| row.starts_with("opencode"))
            .unwrap();
        let title = [&rows_of_space[..2], &rows_of_space[4..running]].concat();
        assert!(title.iter().all(|row| !row.contains('…')), "{title:?}");
        assert_eq!(title.join(" "), format!("{long} {long}"));
        assert_eq!(rows_of_space[running].chars().count(), WIDTH);
        // A long mode no one can switch to keeps its label.
        model.space.as_mut().unwrap().modes.clear();
        let modeless = rows(Some(Section::Space), &model);
        assert!(
            cut_between(&modeless, "Mode: Bartholomew", ""),
            "{modeless:?}"
        );
        // Without environments to change to, the environment's name is cut.
        model.space.as_mut().unwrap().environments.clear();
        let fixed = rows(Some(Section::Space), &model);
        assert!(cut_between(&fixed, "Bartholomew", ""), "{fixed:?}");
    }

    #[test]
    fn everyone_here_is_shown_when_the_server_gives_only_names() {
        let mut model = in_space(ConnectionRole::Owner);
        model.people.clear();
        model.participants = [
            "Ada Lovelace",
            "Bob",
            "Charles Babbage",
            "Dorothy",
            "Edsger",
        ]
        .map(String::from)
        .to_vec();
        let people = rows(Some(Section::PeopleAndSecurity), &model);
        let start = people
            .iter()
            .position(|row| row.starts_with("Here: "))
            .unwrap();
        let here = people[start..start + 2].join(" ");
        assert_eq!(
            here,
            "Here: Ada Lovelace, Bob, Charles Babbage, Dorothy, Edsger"
        );
        assert!(people.iter().all(|row| !row.contains('…')), "{people:?}");
    }

    #[test]
    fn the_rows_fill_the_panel_at_the_menus_font() {
        // A row of `WIDTH` cells fits in the panel, with its background's margin after it, and one more would not.
        let reach = |cells: usize| 2.0 * MARKER + ((cells * CELL_WIDTH) as f32 + 2.0) * PIXEL;
        assert!(reach(WIDTH) <= PANEL);
        assert!(reach(WIDTH + 1) > PANEL);
        assert_eq!(WIDTH, 40);
    }
}
