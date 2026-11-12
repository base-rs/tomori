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
    /// Open pairing on the server for a new device, or close it, as an owner.
    PairingWindow,
    /// Say a new display name with the next press of X, or not after all.
    Rename,
    /// Say the space's new title with the next press of X, or not after all, as an owner.
    RenameSpace,
    /// Let the space's title follow its agent session's again, as an owner.
    FollowSession,
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
    /// Send the agent, away from its home, back home, for everyone.
    AgentHome,
    /// Bring the agent to the participant and pin it there, for everyone.
    CallAgent,
    /// Pin the agent where it stands, for everyone, so that it follows nobody.
    AgentStay,
    /// End the agent's pin, for everyone, so that it follows the speaker again.
    AgentFollow,
    /// Have the agent walk with the participant, keeping with them throughout.
    AgentWalk,
    /// End the agent's walk with the participant.
    AgentWalkEnd,
    /// Step how the agent moves, for everyone, as an owner.
    AgentMovement,
    /// Step how long the agent waits, quiet, before going home, for everyone, as an owner.
    AgentReturn,
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
    /// Step the ambience's level to the next.
    Ambience,
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
    /// Whether the next press of X says the space's new title.
    pub titling: bool,
    pub role: ConnectionRole,
    pub in_space: bool,
    pub participants: Vec<String>,
    pub visitors: usize,
    pub knocks: Vec<KnockRequest>,
    pub can_knock: bool,
    pub knock: Option<KnockState>,
    /// The pairing object's label while pairing is offered: "Pair", or the code while pairing.
    pub pairing: Option<String>,
    /// For an owner whose server says, whether pairing is open on it: `Some(None)` while closed, which offers opening it for a new device, and `Some(Some(seconds))` left while open, which offers closing it. `None` for anyone else. See `design/security.md`.
    pub pairing_window: Option<Option<u32>>,
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
    /// Whether the agent stands away from its home, or is pinned, and this participant may send it home, which offers
    /// sending it home. See `design/environments.md`.
    pub agent_away: bool,
    /// Whether this participant may call the agent to them, which offers **Call Tomori here**: where the space lets it
    /// move, unless it is pinned near them already. See `design/hand-menu.md`.
    pub calls: bool,
    /// What this participant may do of the agent's pin, where the space has it follow the speaker, on a server that
    /// takes pins: `Some(true)` offers **Tomori: stay here**, pinning it where it stands, and `Some(false)` **Tomori:
    /// follow again**, ending its pin. See `design/hand-menu.md`.
    pub pins: Option<bool>,
    /// What this participant may do of the agent walking with them, where the space lets it move, on a server that
    /// takes walks: `Some(true)` offers **Tomori: walk with me**, and `Some(false)`, while it walks with them,
    /// **Tomori: stop walking with me**. See `design/hand-menu.md`.
    pub walks: Option<bool>,
    /// How the agent moves, on a server that says, and whether this participant, an owner, may change it.
    pub movement: Option<(tomori_protocol::AgentMovement, bool)>,
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
    /// The avatar the space's backend names that this build lacks, so that the swarm shows instead, on a row of its
    /// own; `None` otherwise. See `design/avatars.md`.
    pub missing_avatar: Option<String>,
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
    /// Who renamed the space, in place of its agent session's title, if anyone did.
    pub renamed_by: Option<String>,
    /// Whether this participant can rename the space: an owner, on a server that renames spaces.
    pub retitles: bool,
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
    /// Open pairing on the server for a new device, or close it while it is open.
    PairingWindow,
    /// Say a new display name with the next press of X, or not after all.
    Rename,
    /// Say the space's new title with the next press of X, or not after all.
    RenameSpace,
    /// Let the space's title follow its agent session's again.
    FollowSession,
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
    /// Send the agent home, for everyone.
    AgentHome,
    /// Bring the agent here and pin it, for everyone.
    CallAgent,
    /// Pin the agent where it stands, for everyone.
    AgentStay,
    /// Let the agent follow the speaker again, for everyone.
    AgentFollow,
    /// Have the agent walk with this participant.
    AgentWalk,
    /// End the agent's walk with this participant.
    AgentWalkEnd,
    /// Step how the agent moves to the next way, for everyone.
    AgentMovement,
    /// Step how long the agent waits before going home to the next delay, for everyone.
    AgentReturn,
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
    /// Step the ambience's level to the next.
    Ambience,
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
            button(
                &mut items,
                MenuKey::Ambience,
                format!("Ambience: {}", model.feedback.ambience.label()),
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
                // The avatar this build lacks, on a row of its own, its words kept and only its identifier cut.
                if let Some(avatar) = &space.missing_avatar {
                    text(
                        &mut items,
                        MenuKind::Text,
                        row(&[Part::Kept("Unknown avatar: "), Part::Cut(avatar)]),
                    );
                }
                // Who named it, if not its session, and, for an owner, renaming it aloud, since typing on the headset is
                // not practical yet, or letting it follow its session again. See `design/hand-menu.md`.
                if let Some(by) = &space.renamed_by {
                    text(
                        &mut items,
                        MenuKind::Text,
                        row(&[Part::Kept("Renamed by "), Part::Cut(by)]),
                    );
                }
                if space.retitles {
                    button(
                        &mut items,
                        MenuKey::RenameSpace,
                        if model.titling {
                            "Hold X, say the new title".into()
                        } else {
                            "Rename this space".into()
                        },
                    );
                    if space.renamed_by.is_some() {
                        button(
                            &mut items,
                            MenuKey::FollowSession,
                            "Use its agent session's title".into(),
                        );
                    }
                }
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
                // How the agent moves, which only an owner changes; calling it, and sending it home while it is away.
                if model.movement.is_some() || model.agent_away {
                    text(&mut items, MenuKind::Heading, "Tomori, for everyone".into());
                }
                if let Some((movement, owner)) = model.movement {
                    let moves = movement_label(movement.mode);
                    if owner {
                        button(&mut items, MenuKey::AgentMovement, moves.into());
                    } else {
                        text(&mut items, MenuKind::Text, moves.into());
                    }
                    if movement.mode == tomori_protocol::Movement::Follow {
                        let returns = return_label(movement.return_after);
                        if owner {
                            button(&mut items, MenuKey::AgentReturn, returns);
                        } else {
                            text(&mut items, MenuKind::Text, returns);
                        }
                    }
                }
                if model.calls {
                    button(&mut items, MenuKey::CallAgent, "Call Tomori here".into());
                }
                match model.pins {
                    Some(true) => {
                        button(&mut items, MenuKey::AgentStay, "Tomori: stay here".into())
                    }
                    Some(false) => {
                        button(
                            &mut items,
                            MenuKey::AgentFollow,
                            "Tomori: follow again".into(),
                        );
                    }
                    None => {}
                }
                match model.walks {
                    Some(true) => button(
                        &mut items,
                        MenuKey::AgentWalk,
                        "Tomori: walk with me".into(),
                    ),
                    Some(false) => button(
                        &mut items,
                        MenuKey::AgentWalkEnd,
                        "Tomori: stop walking with me".into(),
                    ),
                    None => {}
                }
                if model.agent_away {
                    button(&mut items, MenuKey::AgentHome, "Send Tomori home".into());
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
            // An owner opens pairing for a new device here, which closes again by itself; see `design/security.md`.
            if let Some(window) = model.pairing_window {
                let label = match window {
                    None => "Pair a new device".to_owned(),
                    Some(0) => "Pairing open: close it".to_owned(),
                    Some(seconds) => format!(
                        "Pairing open, {}:{:02} left: close it",
                        seconds / 60,
                        seconds % 60
                    ),
                };
                button(&mut items, MenuKey::PairingWindow, label);
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
            MenuKey::AgentHome => Some(Choice::AgentHome),
            MenuKey::CallAgent => Some(Choice::CallAgent),
            MenuKey::AgentStay => Some(Choice::AgentStay),
            MenuKey::AgentFollow => Some(Choice::AgentFollow),
            MenuKey::AgentWalk => Some(Choice::AgentWalk),
            MenuKey::AgentWalkEnd => Some(Choice::AgentWalkEnd),
            MenuKey::AgentMovement => Some(Choice::AgentMovement),
            MenuKey::AgentReturn => Some(Choice::AgentReturn),
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
            MenuKey::Ambience => Some(Choice::Ambience),
            MenuKey::Knock => Some(Choice::Knock),
            MenuKey::Admit(id) => Some(Choice::Admit(id.clone())),
            MenuKey::Decline(id) => Some(Choice::Decline(id.clone())),
            MenuKey::Pair => Some(Choice::Pair),
            MenuKey::PairingWindow => Some(Choice::PairingWindow),
            MenuKey::Rename => Some(Choice::Rename),
            MenuKey::RenameSpace => Some(Choice::RenameSpace),
            MenuKey::FollowSession => Some(Choice::FollowSession),
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

/// How the agent moves, as its row in the Space section says it.
fn movement_label(mode: tomori_protocol::Movement) -> &'static str {
    match mode {
        tomori_protocol::Movement::Follow => "Moves: follows the speaker",
        tomori_protocol::Movement::Home => "Moves: stays home",
        _ => "Moves: only when asked",
    }
}

/// How long a following agent waits, quiet, before going home, as its row in the Space section says it.
fn return_label(seconds: Option<u32>) -> String {
    match seconds {
        None => "Goes home: never".into(),
        Some(seconds) if seconds < 120 => format!("Goes home after {seconds} s quiet"),
        Some(seconds) => format!("Goes home after {} min quiet", seconds / 60),
    }
}

#[cfg(test)]
mod tests;
