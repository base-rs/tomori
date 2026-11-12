// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The frame step that the headset and the desktop window share: the controller's buttons, arrival, the hand menu, the right wrist's transcript, pointing and choosing, the swarm, the space's workers and the shared panels, from plain input. Each platform's own behaviour is in [`headset`] and [`desktop`], which turn their devices' input into [`Buttons`] and [`Senses`]; the shells in `xr.rs` and `desktop_participant.rs` make only device calls. What a frame draws is [`Frame`], which the renderer applies. This is the platform-neutral core that `design/clients.md` asks for.
#[cfg(not(target_os = "android"))]
pub mod desktop;
pub mod headset;
mod precedence;
use precedence::Ladder as _;

use std::time::Instant;

use glam::{Mat4, Vec2, Vec3};

use crate::{
    arrival, arrival_motion,
    cards::CardNavigation,
    controller::Pose,
    controller_ui::{self, Indicator, Moment, Sheet, Wrist, WristFrame},
    conversation::Conversation,
    delegation,
    hand_menu::{self, HandMenu, Palm},
    hands::Hand,
    interaction,
    locomotion::{self, Rig, Travel},
    panel_anchor::PanelAnchor,
    pointing::{Pointer, Ray},
    recording::Trackers,
    sides::{Hands, Side},
    swarm::Swarm,
};
use tomori_client::feedback::{self, Feedback};

/// How far a stick must move before it counts.
const STICK: f32 = 0.6;
/// How far the right trigger is pulled before the right palm counts as pointing, which keeps the transcript from showing.
const POINTING: f32 = 0.2;

/// One frame of the controller's buttons: presses since the last frame, and what is held.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Buttons {
    /// A was pressed.
    pub a: bool,
    /// A is held. With a press, it is what lets an environment's select be let go once A is. A press that is not held
    /// is one let go within the frame, which is let go on the next.
    pub a_held: bool,
    /// B was pressed.
    pub back: bool,
    /// B is held.
    pub back_held: bool,
    /// Y was pressed.
    pub y: bool,
    /// X is held: push-to-talk.
    pub talk: bool,
    /// The desktop's V was pressed: mute spoken replies, as the hand menu's row does. The headset's left stick click
    /// toggles the live microphone instead, and sets nothing here.
    pub mute: bool,
    /// The right stick was clicked: replay the latest reply.
    pub replay: bool,
    pub left_stick: Vec2,
    pub right_stick: Vec2,
    /// How many rows the mouse's wheel scrolled the panel pointed at since the last frame: down, or up where fewer
    /// than none. Pointing at an environment's object that takes the right stick, it pushes the object as the stick
    /// does, as far as it would scroll a panel, at most at the stick's rate. A headset scrolls with the right stick
    /// instead.
    pub scroll: f32,
}

/// How the attractor is drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Marker {
    /// Always, brighter while it attracts: the right controller, while neither its model nor the right hand is drawn.
    Always,
    /// Only while it attracts, as the mouse's point.
    #[default]
    WhileAttracting,
    /// Never: the right controller's model or hand is drawn instead.
    Hidden,
}

/// Where things are this frame, in the scene, from each platform's own logic.
#[derive(Clone, Copy, Debug, Default)]
pub struct Senses {
    /// The participant has input: the session is focused, or the window is.
    pub focused: bool,
    /// The head's transform.
    pub head: Option<Mat4>,
    /// The head as a pose: where the participant hears from and is seen by the others.
    pub head_pose: Option<Pose>,
    /// The point that what the hands hold faces, which stays put as the head turns.
    pub viewer: Option<Vec3>,
    /// The head in the eyes' space, which the floating panel follows.
    pub space_head: Mat4,
    /// The pointing ray: the right controller's aim, or the mouse's.
    pub ray: Option<Ray>,
    /// The trigger, while it can act; see [`Pointer::update`].
    pub trigger: Option<f32>,
    /// The left controller's aim ray, its trigger and its grip, which act on Tomori's own things as the right's do; the
    /// desktop has none. See `design/interaction.md`.
    pub left_ray: Option<Ray>,
    pub left_trigger: Option<f32>,
    pub left_grip: Option<f32>,
    /// Where the swarm is attracted to while the left trigger attracts it, outside a space.
    pub left_attractor: Option<Vec3>,
    /// The hand whose ray aims a teleport: the left on the headset, the hand whose stick aims it, and the right, the
    /// mouse, on the desktop. See `design/locomotion.md`.
    pub teleport: Side,
    /// The right hand points, which keeps the transcript from showing.
    pub pointing: bool,
    /// The left palm, which shows the hand menu while it faces up.
    pub left_palm: Option<Palm>,
    /// The right palm, which shows the transcript while it faces up.
    pub right_palm: Option<Palm>,
    /// The right palm is a tracked hand without a controller, so the transcript only reads.
    pub hands_only: bool,
    /// The right grip, which detaches the transcript.
    pub grip: Option<f32>,
    /// Where the recording indicator can stand.
    pub trackers: Trackers,
    /// Where the swarm is attracted to, while the trigger attracts.
    pub attractor: Option<Vec3>,
    pub marker: Marker,
    /// The controllers' poses, for the others to see.
    pub controllers: [Option<Pose>; 2],
    /// The tracked hands, for the others to see.
    pub hands: [Option<Hand>; 2],
    /// Cue on the palms that something waits, while their panels are closed.
    pub cues: bool,
    /// Draw the pointing ray.
    pub beam: bool,
}

/// Arrival's objects to draw, with what the ray meets and the ray.
#[derive(Clone, Debug, PartialEq)]
pub struct Arrival {
    pub scene: arrival::Scene,
    pub hovered: Option<arrival::Key>,
    /// The right ray and the left, as drawn. See `design/interaction.md`.
    pub ray: Option<crate::pointing::Beam>,
    pub left_ray: Option<crate::pointing::Beam>,
    pub time: f32,
}

/// This participant's head, controllers and hands, for the others in the space, and the panels they carry and which of
/// their rays are shown, for the server alone.
#[derive(Clone, Debug)]
pub struct Presence {
    pub head: Pose,
    pub left: Option<Pose>,
    pub right: Option<Pose>,
    pub hands: [Option<Hand>; 2],
    /// The panels carried, by their identities, the right hand's first, so that the space's agent does not move them
    /// out of the hands.
    pub carried: Vec<String>,
    /// Which rays are shown, left then right, so that the space's agent is told what each shown one points at.
    pub rays: [bool; 2],
}

impl Presence {
    /// Report it to the server through `connection`, so the others in the space can render it.
    pub fn send(&self, connection: &mut tomori_client::connection::Connection) {
        connection.send_presence(
            self.head.into(),
            self.left.map(Into::into),
            self.right.map(Into::into),
            self.hands,
            &self.carried,
            self.rays,
        );
    }
}

/// The shared panels this frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Panels {
    /// The status pill, in the eyes' space.
    pub pill: Sheet,
    /// The recording indicator and the `LIVE` badge, in the scene.
    pub indicator: [Sheet; 2],
    /// The right wrist's transcript, in the scene.
    pub wrist: Sheet,
}

/// What one frame draws and reports.
pub struct Frame<'a> {
    pub scenery: arrival_motion::Scenery,
    pub swarm: &'a Swarm,
    /// Where the attractor is drawn.
    pub marker: Option<Vec3>,
    pub attracting: bool,
    /// The result card shown, and its page.
    pub card: usize,
    pub content: usize,
    pub cubes: &'a [delegation::Cube],
    pub stations: &'a [delegation::Station],
    /// The space's timers' dials, their cubes among `cubes`, for their signs. See `design/timers.md`.
    pub dials: &'a [crate::dials::Dial],
    /// Where this client's viewer is, followed smoothly, which workers' signs turn to face; nowhere without one, when
    /// they face whoever the agent faces. See [`crate::sign_facing`].
    pub sign_facing: Option<Vec3>,
    pub workspace: &'a delegation::Workspace,
    /// Arrival's objects, while there are any to draw.
    pub arrival: Option<Arrival>,
    pub panels: Panels,
    /// Where this participant hears from, and where the agent is.
    pub listener: Option<Pose>,
    pub agent: [f32; 3],
    /// What to tell the others in the space; visitors send nothing.
    pub presence: Option<Presence>,
    /// The GitHub environment's inbox and cards, in the scene, while the space shows them.
    pub cards: Vec<controller_ui::Sheet>,
    /// The space's projection walls and panels, farthest first, the label naming the panel pointed at, and the hint
    /// beside a panel carried, in the scene. See `design/projection.md`.
    pub projection: Vec<controller_ui::Sheet>,
    /// The images those panels show that the client holds, by hash, each a PNG for the renderer to decode, once each
    /// however many panels show it. See `design/projection.md`.
    pub images: Vec<(String, std::sync::Arc<[u8]>)>,
    /// The space's volumes, whose solids are drawn among the opaque bodies; their labels are among `projection`. See
    /// `design/diagrams.md`.
    pub volumes: Vec<crate::projection::volume::Drawn>,
    /// Where a teleport being aimed would land, which is drawn as a ring on the surface; see `design/locomotion.md`.
    pub landing: Option<Vec3>,
    /// How dark the view is drawn, from 0 to 1, as it comes back after a teleport; see `design/locomotion.md`.
    pub blink: f32,
    /// The teleport pads of the space's profile, drawn once its scenery has grown in, the one pointed at highlighted;
    /// see `design/locomotion.md`.
    pub pads: Vec<crate::pads::Shown>,
    /// What the environment's client side shows, for its scenery to draw. See `design/environments.md`.
    pub environment: Option<crate::client_side::Drawn>,
    /// The carriage from the environment's own coordinates, in which its scenery draws, into the space's. See
    /// `design/environments.md`.
    pub carriage: crate::anchors::Anchor,
    /// The registered avatar shown, while the swarm is not, and what it shows, for its scenery to draw. See
    /// `design/avatars.md`.
    pub avatar: Option<crate::avatar::Shown<'a>>,
    /// The ring drawn on the floor at home, which stays there wherever the agent goes. See `design/environments.md`.
    pub home_ring: crate::anchors::HomeRing,
}

/// Whose a push of a stick is: each push belongs to what took it as it left the middle, until it returns there, as
/// [`precedence::Right`] orders the right stick's and [`precedence::Left`] the left's. Each stick acts on what its own
/// hand carries and its own hand's ray is on.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
enum Push {
    /// At the middle.
    #[default]
    Free,
    /// A panel or a wall was carried in the stick's hand as it left the middle: the push moves and turns it.
    Carried,
    /// Nothing was carried in the stick's hand, and its ray, shown in full, pointed at this panel, which has more than it
    /// shows: the push scrolls it, for this participant alone.
    Scrolled(String),
    /// In a space whose environment allows travel here, nothing the stick's ray was on took it: the push travels, turning,
    /// aiming a teleport along that hand's ray, stepping back or rising. See `design/locomotion.md`.
    Travel,
    /// The transcript showed, or the ray was on something else of Tomori's, or on nothing with no environment's client
    /// side to take it, or the other stick travelled, scrolled or pushed an object already: the push moves the
    /// transcript's focus or pages cards, as ever, or, of the left stick outside a space, moves in what the lobby lists.
    Other,
    /// Nothing of Tomori's took it, in a space with an environment's client side: the push is the client side's. See
    /// `design/environments.md`.
    Environment,
    /// Nothing of Tomori's took it, and the stick's ray pointed at this object of the client side's, which takes the
    /// stick: the push is the object's, told as the client side's `Input::scroll` with the push's number, which no other
    /// push shares. See `design/environments.md`.
    Object(String, u64),
    /// The push was an object's when focus was lost or its client side went: it is nobody's until it returns to the
    /// middle, telling the next client side nothing.
    Abandoned,
}

/// The rungs of either stick's order whose push is something's own, as a push leaves the middle: see [`Push`].
enum SharedRung {
    Carried,
    Scroll(String),
    Object(String),
    Travel,
    Environment,
}

impl Push {
    /// Whose the push is with the stick at `stick`: the owner `claim` names, as it leaves the middle, or else whose it
    /// was.
    fn update(self, stick: Vec2, claim: impl FnOnce() -> Self) -> Self {
        // A stick that reads nonsense is at the middle.
        if !stick.is_finite() || stick.length() < STICK_CENTRE {
            return Self::Free;
        }
        match self {
            Self::Free => claim(),
            owner => owner,
        }
    }
}

/// Whose a squeeze of the right grip is: each squeeze belongs to what took it as it closed, until it is let go, as
/// [`precedence::Grab`] orders. Tomori's own uses of the grip each decide for themselves what they take, as before.
#[derive(Clone, Debug, Default, PartialEq)]
enum Gripping {
    /// Let go.
    #[default]
    Free,
    /// Something of Tomori's, or nothing, took it: it is theirs, as ever, and the environment's client side is told
    /// nothing of it.
    Other,
    /// It took the environment's object `object`, within reach of the hand, or pointed at `distance` along the ray,
    /// which the hand is held at where there is no controller, as on the desktop; `told` once its client side has been
    /// told it began, and the hand it was last told of.
    Environment {
        object: String,
        distance: Option<f32>,
        told: bool,
        hand: Option<crate::client_side::Hand>,
    },
    /// It was the environment's when its client side went: it is nobody's until it is let go, telling the next client
    /// side nothing.
    Abandoned,
}

/// The client side of the joined space's environment, for that space and environment, with what the participant does
/// with it. See `design/environments.md`.
struct Client {
    space: String,
    environment: crate::environment::Environment,
    side: Box<dyn crate::client_side::ClientSide>,
    /// The selectable object of its that the ray pointed at last frame, which the trigger and B act on.
    focus: Option<String>,
    /// What the trigger and B did for it since its last frame, which it is told on its next.
    select: Option<crate::client_side::Select>,
    back: bool,
    /// The object of its that each ray pointed at last frame, if it takes the stick, which a push of that hand's stick
    /// takes.
    scrollable: Hands<Option<String>>,
    /// The pushes of the sticks, and turns of the wheel, it has been told of.
    scroller: crate::client_side::Scroller,
    /// The object of its within reach of each hand last frame, and the one each ray pointed at last frame that takes the
    /// grip, with how far along the ray, which a squeeze of that hand's grip takes, the one within reach first.
    reachable: Hands<Option<String>>,
    grabbable: Hands<Option<(String, f32)>>,
}

/// The index of `side`'s controller among the platform's.
fn controller_of(side: Side) -> usize {
    match side {
        Side::Left => crate::controller::LEFT,
        Side::Right => crate::controller::RIGHT,
    }
}

/// What a client side is told this frame of a squeeze of the grip of `side` that took an object of its, as `gripping`
/// says, with the hand at `hand`, if it is tracked, or else `along` the ray as far as it took the object, and the grip
/// `closed` or not: `Began` once there is a hand to tell of, `Changed` while it is held and `Ended` once as it is let go,
/// with the hand last told while tracking is lost; and nothing of its end if its beginning never was.
fn told(
    gripping: &mut Gripping,
    hand: Option<crate::client_side::Hand>,
    along: impl FnOnce(f32) -> Option<crate::client_side::Hand>,
    closed: bool,
    side: Side,
) -> Option<crate::client_side::Grab> {
    let Gripping::Environment {
        object,
        distance,
        told,
        hand: last,
    } = std::mem::take(gripping)
    else {
        return None;
    };
    let held = hand.or_else(|| distance.and_then(along)).or(last);
    let phase = match (closed, told) {
        (true, false) => crate::client_side::Phase::Began,
        (true, true) => crate::client_side::Phase::Changed,
        (false, _) => crate::client_side::Phase::Ended,
    };
    // Told nothing until there is a hand to tell of, nor its end if its beginning never was.
    let told_now = held.filter(|_| closed || told);
    if closed {
        *gripping = Gripping::Environment {
            object: object.clone(),
            distance,
            told: told || told_now.is_some(),
            hand: held,
        };
    }
    told_now.map(|hand| crate::client_side::Grab::new(object, hand, phase).of(side.into()))
}

/// How near the middle the right stick returns to end a push.
const STICK_CENTRE: f32 = 0.2;

/// The most rows of a panel a turn of the wheel that an object takes keeps to push it by: as far as the stick scrolls
/// a panel in 0.4 seconds, two notches, so that a fast spin ends soon after the wheel stops. See `design/desktop.md`.
const WHEEL_BACKLOG: f32 = crate::projection::SCROLL_SPEED * 0.4;

/// How long a turn of the wheel that an object takes stays one push once it has pushed all it was turned, in seconds,
/// so that a slow scroll of a touchpad, a fraction of a notch a frame, is one push rather than many.
const WHEEL_LINGER: f32 = 0.1;

/// A turn of the desktop's wheel that an object takes, as a push of the right stick: the rows it was turned, which the
/// push scrolls as the stick would a panel, at most [`crate::projection::SCROLL_SPEED`] rows a second, for as long as
/// they last and [`WHEEL_LINGER`] more.
#[derive(Debug)]
struct WheelPush {
    object: String,
    /// The push's number, which no other push shares.
    number: u64,
    /// The rows still to push, forward, as the stick forward scrolls a panel up, where more than none, at most
    /// [`WHEEL_BACKLOG`] either way.
    rows: f32,
    /// How long it has had no rows to push, in seconds.
    idle: f32,
}

impl WheelPush {
    fn new(object: &str, number: u64) -> Self {
        Self {
            object: object.to_owned(),
            number,
            rows: 0.0,
            idle: 0.0,
        }
    }

    /// The wheel turned `scrolled` rows, down where more than none, as it scrolls a panel.
    fn turn(&mut self, scrolled: f32) {
        self.rows = (self.rows - scrolled).clamp(-WHEEL_BACKLOG, WHEEL_BACKLOG);
    }

    /// The stick's axis for a frame of `dt` seconds: as far forward or back as scrolls a panel the rows still to push,
    /// at most all the way, or zero once there are none, or `None` once it has had none for [`WHEEL_LINGER`], when the
    /// push ends.
    fn advance(&mut self, dt: f32) -> Option<Vec2> {
        let dt = if dt.is_finite() { dt.max(0.0) } else { 0.0 };
        if self.rows == 0.0 {
            self.idle += dt;
            return (self.idle < WHEEL_LINGER).then_some(Vec2::ZERO);
        }
        self.idle = 0.0;
        let reach = crate::projection::SCROLL_SPEED * dt;
        if reach == 0.0 {
            return Some(Vec2::ZERO);
        }
        let forward = (self.rows / reach).clamp(-1.0, 1.0);
        self.rows = if forward.abs() < 1.0 {
            0.0
        } else {
            self.rows - forward * reach
        };
        Some(Vec2::new(0.0, forward))
    }
}

/// The panel to scroll this frame, and by how many rows: the one the right stick scrolls, `stick`, pushed so far
/// forward, for `dt` seconds, at [`crate::projection::scroll_rate`], and the rows the wheel scrolled since the last
/// frame, `wheel`, of the same panel or, while the stick scrolls none, of another.
fn scrolled(
    stick: Option<&(String, f32)>,
    wheel: Option<(String, f32)>,
    dt: f32,
) -> Option<(String, f32)> {
    let stick = stick
        .map(|(panel, forward)| (panel.clone(), crate::projection::scroll_rate(*forward) * dt));
    match (stick, wheel) {
        (Some((panel, rows)), Some((wheeled, more))) if panel == wheeled => {
            Some((panel, rows + more))
        }
        (Some(stick), _) => Some(stick),
        (None, wheel) => wheel,
    }
}

/// Where travel applies: the joined space, by its identifier, and its environment as drawn.
fn place(conversation: &Conversation) -> locomotion::Place<'_> {
    conversation
        .joined_space()
        .map(|(space, _)| (space.id.as_str(), conversation.environment()))
}

/// The scene arrival heads for: its objects outside a space, and inside one, none, with the space's environment to grow out of its object.
pub fn arrival_target(conversation: &Conversation) -> arrival::Scene {
    match conversation.arrival() {
        Some(view) => arrival::Scene::layout(&view),
        None => arrival::Scene::entered(conversation.entered(), conversation.environment()),
    }
}

/// Everything the frame step keeps between frames.
pub struct Step {
    stick_latched: bool,
    /// When B was pressed, while it is held: holding it leaves a space.
    back_since: Option<Instant>,
    was_talking: bool,
    /// After focus loss, X must be released before it records again, so that holding it through a system menu does not start a recording.
    talk_blocked: bool,
    /// Whether the hand menu showed last frame, when A is read, so that A then chooses only in it.
    menu_was_shown: bool,
    hand_menu: HandMenu,
    wrist: Wrist,
    indicator: Indicator,
    /// What each hand's ray points at, chooses and takes hold of. See `design/interaction.md`.
    hands: Hands<HandState>,
    /// The others' hands, drawn between the presence updates that bring them, for the environment's client side.
    relayed: Hands<crate::relayed::Relayed>,
    /// The hand whose trigger began drawing the swarm first, outside a space, while either does.
    attracting: Option<Side>,
    /// The agent as this client shows it: the swarm, or the avatar the joined space's backend names. See
    /// `design/avatars.md`.
    coordinator: crate::avatar::Coordinator,
    signals: interaction::Signals,
    motion: arrival_motion::Motion,
    arrival_time: f32,
    delegation_scene: delegation::Scene,
    workers: delegation::Workers,
    /// Where workers' signs face in this client.
    sign_facing: crate::sign_facing::SignFacing,
    presentation_navigation: CardNavigation,
    /// In a space, the panel stands in the world and follows lazily; see `design/interaction.md`.
    panel: PanelAnchor,
    /// The GitHub environment's cards: the one carried and the one put down; see `design/github.md`.
    board: crate::board::Board,
    /// The space's projected panels: the one carried and the one let go; see `design/projection.md`.
    projector: crate::projection::Projector,
    /// Where the participant has travelled, in an environment that allows it, and the teleport being aimed; see `design/locomotion.md`.
    travel: Travel,
    /// The stick whose push travels in such a space, from the last buttons, and whether it is the right, so that a
    /// teleport it aims is aimed along the right ray, until the push has been followed to its end.
    travel_stick: Vec2,
    travel_right: bool,
    /// Whose the right stick's push is, from when it left the middle until it returns.
    right_stick: Push,
    /// The stick whose push is an object's, from the last buttons.
    scroll_axis: Vec2,
    /// A turn of the desktop's wheel that an object takes, as a push of the right stick. See `design/desktop.md`.
    wheel_push: Option<WheelPush>,
    /// How many pushes have been an object's, which numbers each.
    pushes: u64,
    /// Each stick, while what its hand carries takes it, from the last buttons, and whether the right stick was clicked
    /// since the last frame, which turns the panel back to square.
    carry_stick: Hands<Vec2>,
    straighten: bool,
    /// The panel a stick scrolls, and how far forward it is pushed, from the last buttons, and the panel the
    /// mouse's wheel scrolled since the last frame, and by how many rows.
    scroll_stick: Option<(String, f32)>,
    wheel: Option<(String, f32)>,
    /// The wall carried as B was pressed, and when: removed for everyone, or asked to confirm, if B is let go within
    /// [`crate::conversation::DISMISS_PRESS`] while it is still carried. See `design/projection.md`.
    removing: Option<(String, Instant)>,
    /// The panel carried as B was pressed, and when: thrown away for everyone if B is let go within
    /// [`crate::conversation::DISMISS_PRESS`] while it is still carried. See `design/projection.md`.
    throwing: Option<(String, Instant)>,
    /// The controls the hint beside a panel carried names.
    controls: crate::projection::Controls,
    /// The client side of the joined space's environment, for that space and environment. See `design/environments.md`.
    client: Option<Client>,
    /// The space, environment and carriage the ambience last handed to the feedback model was for, while one plays.
    /// See `design/spatial-audio.md`.
    ambience: Option<(
        String,
        crate::environment::Environment,
        crate::anchors::Anchor,
    )>,
    /// The left and right sticks, where they are the client side's, from the last buttons.
    environment_sticks: (Vec2, Vec2),
    /// Whose the left stick's push is, from when it left the middle until it returns.
    left_stick: Push,
    /// The pulls of the trigger and the presses of A on the client side's selectable objects, which outlive a client
    /// side, so that a pull begun for one is nobody's once it has gone. See `design/environments.md`.
    selector: crate::client_side::Selector,
    /// What A did, from the last buttons, which the selector is told with the trigger.
    a_press: crate::client_side::Press,
    /// Whose the last press of B was, for tests that check the order.
    #[cfg(test)]
    last_back: Option<crate::conversation::Back>,
    /// Whose the last squeeze of a grip was, and which hand's, for tests that check the order.
    #[cfg(test)]
    last_grab: Option<(Side, precedence::Grab)>,
    /// The agent's home as the swarm last stood at it: when it changes, as a space opens or changes environment, the
    /// swarm appears at the new one rather than gliding. See `design/environments.md`.
    agent_home: crate::anchors::Anchor,
    /// This client's clock, which the timers' dials count down by, reckoned from the instants each frame is given and
    /// read again from the system's clock as they drift apart, and the system's clock, unless a test gives another. See
    /// `design/timers.md`.
    clock: crate::dials::Clock,
    system: Option<Box<dyn Fn() -> f64 + Send + Sync>>,
    /// The rig the platform placed this frame's poses with, as [`Self::rig`] last settled it.
    placed: Rig,
    /// Where the space's environment stood in the participant's room on the last frame in a space, and, once it is
    /// left, while it shrinks back into its object: it stays there until it has gone. See `design/arrival.md`.
    standing: Option<Standing>,
    /// The space joined and the rig as of the last frame, which tell joining and leaving, as the swarm flies between
    /// the lobby and the space, and the rig going home from travel. See `design/arrival.md`.
    seen_space: Option<String>,
    seen_rig: Rig,
}

/// Where a space's environment stands in the participant's room: the environment drawn, its carriage and the ring on the
/// floor at home, in the room.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Standing {
    environment: crate::environment::Environment,
    carriage: crate::anchors::Anchor,
    /// The ring at home, its middle and its height in the participant's room.
    ring: crate::anchors::HomeRing,
}

/// The home ring `ring`, its middle and the height it is drawn at carried by `carry`, as the rig carries points.
fn carried_ring(
    ring: crate::anchors::HomeRing,
    carry: impl Fn(Vec3) -> Vec3,
) -> crate::anchors::HomeRing {
    crate::anchors::HomeRing {
        centre: carry(ring.centre),
        height: carry(ring.centre.with_y(ring.height)).y,
        ..ring
    }
}

/// An unturned anchor at `position`, which the rig carries as it carries the environment.
fn at(position: Vec3) -> crate::anchors::Anchor {
    crate::anchors::Anchor {
        position,
        orientation: glam::Quat::IDENTITY,
    }
}

/// What one hand's ray points at, chooses and takes hold of, across frames: the same for either hand, the right's
/// alone acting on the environment's objects for now. See `design/interaction.md`.
struct HandState {
    /// Arrival's objects and the hand menu's rows, which the ray points at and the trigger chooses.
    pointer: Pointer<arrival::Key>,
    /// The arrival object or menu row the ray met on the last frame, which A chooses.
    highlighted: Option<arrival::Key>,
    hover: feedback::Hover,
    /// The choosable row of a list the ray is on, which ticks as it is entered, apart from its panel, so that leaving it
    /// for the rest of the panel ticks nothing.
    row_hover: feedback::Hover,
    /// What the ray pointed at last frame, Tomori's own interface first, which decides whose the hand's trigger and grip
    /// are as they begin, and the buttons and sticks', the right's first. See `design/interaction.md`.
    aim: precedence::Aim,
    /// The panel the ray pointed at last frame, not one carried, which a push of the right stick or the wheel scrolls.
    panel: Option<String>,
    /// The checklist item or row of a list the ray met last frame, by its panel and line.
    item: Option<(String, u32)>,
    /// How far along the ray the card it met last frame was, which the grip picks up instead of a panel behind it.
    card_reach: f32,
    /// The pad the ray pointed at last frame, which a pull begun on it, or A, chooses, and the pulls of the trigger on
    /// pads, with the thresholds that choose an arrival object. See `design/locomotion.md`.
    pad: Option<crate::pads::Way>,
    pad_selector: crate::client_side::Selector,
    /// The pulls of the trigger on choosable rows of an environment's lists, with the same thresholds, a pull begun on
    /// one choosing it once pressed there. See `design/panel-actions.md`.
    row_selector: crate::client_side::Selector,
    /// The label on the ray last drawn, handed over again while it says the same, so that it is not drawn again.
    ray_label: crate::projection::KeptLabel,
    /// Whose the grip's squeeze is, from when it closed until it is let go, and whether it is closed.
    gripping: Gripping,
    grip_closed: bool,
    /// What the ray is on, as words begun would name it, and when it moved onto it, so that words name what was pointed
    /// at most recently first.
    target: Option<tomori_protocol::Resource>,
    since: Option<Instant>,
    /// The ray as drawn: when it last had something to say, which keeps it drawn in full a second longer, and whether
    /// its hand points down, as an arm resting at the side does. See `design/interaction.md`.
    lit: Option<Instant>,
    lowered: bool,
    /// The ray was drawn in full last frame, rather than as a stub or hidden, as its hand hangs at the side: only then
    /// does what it is on capture a push of its hand's stick. See `design/interaction.md`.
    full: bool,
}

impl Default for HandState {
    fn default() -> Self {
        Self {
            pointer: Pointer::default(),
            highlighted: None,
            hover: feedback::Hover::default(),
            row_hover: feedback::Hover::default(),
            aim: precedence::Aim::Nothing,
            panel: None,
            item: None,
            card_reach: f32::INFINITY,
            pad: None,
            pad_selector: crate::client_side::Selector::default(),
            row_selector: crate::client_side::Selector::default(),
            ray_label: crate::projection::KeptLabel::default(),
            gripping: Gripping::Free,
            grip_closed: false,
            target: None,
            since: None,
            lit: None,
            lowered: false,
            full: false,
        }
    }
}

/// How long a ray stays drawn in full after it last had something to say. See `design/interaction.md`.
const LIT: std::time::Duration = std::time::Duration::from_secs(1);
/// How long a ray that has nothing to say is drawn, from its controller, in metres.
pub const STUB: f32 = 0.15;
/// How far below the horizontal a hand points, as the sine of the angle, for its ray to be hidden, 50°, and to show
/// again, 45°, so that a hand held near the edge does not flicker.
const LOWERED: f32 = 0.766;
const RAISED: f32 = 0.707;

impl Default for Step {
    fn default() -> Self {
        Self {
            stick_latched: false,
            back_since: None,
            was_talking: false,
            talk_blocked: false,
            menu_was_shown: false,
            hand_menu: HandMenu::default(),
            wrist: Wrist::default(),
            indicator: Indicator::default(),
            hands: Hands::default(),
            relayed: Hands::default(),
            attracting: None,
            coordinator: crate::avatar::Coordinator::default(),
            signals: interaction::Signals::default(),
            motion: arrival_motion::Motion::default(),
            arrival_time: 0.0,
            delegation_scene: delegation::Scene::new(),
            workers: delegation::Workers::default(),
            sign_facing: crate::sign_facing::SignFacing::default(),
            presentation_navigation: CardNavigation::default(),
            panel: PanelAnchor::default(),
            board: crate::board::Board::default(),
            projector: crate::projection::Projector::paced(),
            travel: Travel::default(),
            travel_stick: Vec2::ZERO,
            travel_right: false,
            right_stick: Push::Free,
            scroll_axis: Vec2::ZERO,
            wheel_push: None,
            pushes: 0,
            carry_stick: Hands::default(),
            straighten: false,
            scroll_stick: None,
            wheel: None,
            throwing: None,
            removing: None,
            controls: crate::projection::Controls::default(),
            client: None,
            ambience: None,
            environment_sticks: (Vec2::ZERO, Vec2::ZERO),
            left_stick: Push::Free,
            selector: crate::client_side::Selector::default(),
            a_press: crate::client_side::Press::default(),
            #[cfg(test)]
            last_back: None,
            #[cfg(test)]
            last_grab: None,
            agent_home: crate::anchors::Anchor::AGENT,
            clock: crate::dials::Clock::default(),
            system: None,
            placed: Rig::default(),
            standing: None,
            seen_space: None,
            seen_rig: Rig::default(),
        }
    }
}

impl Step {
    /// Focus is lost: nothing held carries over, and an utterance being recorded is cancelled.
    pub fn blur(&mut self, conversation: &mut Conversation) {
        self.stick_latched = false;
        self.back_since = None;
        conversation.hold_back(None);
        self.was_talking = false;
        self.talk_blocked = true;
        conversation.stop_live();
        conversation.stop_talking();
        for side in Side::BOTH {
            let hand = &mut self.hands[side];
            hand.pointer.release();
            hand.highlighted = None;
            hand.pad = None;
        }
        self.attracting = None;
        self.presentation_navigation.release();
        self.cancel_travel();
        // A push that was an object's is nobody's until it returns to the middle, so that it does not begin again.
        self.abandon_scroll();
        for push in [&mut self.right_stick, &mut self.left_stick] {
            if *push != Push::Abandoned {
                *push = Push::Free;
            }
        }
        self.carry_stick = Hands::default();
        self.straighten = false;
        self.scroll_stick = None;
        self.wheel = None;
        self.throwing = None;
        self.removing = None;
        self.environment_sticks = (Vec2::ZERO, Vec2::ZERO);
        self.a_press = crate::client_side::Press::default();
        if let Some(client) = self.client.as_mut() {
            client.back = false;
        }
    }

    /// A push of the right stick or a turn of the wheel that is an object's ends, as focus is lost or its client side
    /// goes: the push is nobody's until it returns to the middle, and the client side, if it is still there, is told it
    /// ended on its next frame.
    fn abandon_scroll(&mut self) {
        for push in [&mut self.right_stick, &mut self.left_stick] {
            if matches!(push, Push::Object(..)) {
                *push = Push::Abandoned;
            }
        }
        self.wheel_push = None;
        self.scroll_axis = Vec2::ZERO;
        // And a squeeze of a grip that was an object's, which the next client side is told nothing of.
        for side in Side::BOTH {
            if matches!(self.hands[side].gripping, Gripping::Environment { .. }) {
                self.hands[side].gripping = Gripping::Abandoned;
            }
        }
    }

    /// Whose `side`'s grip's squeeze is this frame, from `grip`, its value while it acts, with `wrist_active` whether
    /// the transcript showed: as it closes, the first rung of [`precedence::Grab`] that claims it for that hand, as
    /// things stood last frame, Tomori's own first, the environment's object within reach of the hand, or else pointed
    /// at with its ray, only for a participant; held until it is let go. Returns whether it is
    /// the environment's, or was as it was let go this frame, so that nothing of Tomori's is given it.
    fn grip(
        &mut self,
        side: Side,
        grip: Option<f32>,
        wrist_active: bool,
        participates: bool,
    ) -> bool {
        let value = grip.filter(|value| value.is_finite()).unwrap_or(0.0);
        let hand = &self.hands[side];
        let closed = if hand.grip_closed {
            value > controller_ui::RELEASE
        } else {
            value > controller_ui::SQUEEZE
        };
        let began = closed && !hand.grip_closed;
        self.hands[side].grip_closed = closed;
        if began && self.hands[side].gripping == Gripping::Free {
            let target = self
                .client
                .as_ref()
                .filter(|_| participates)
                .and_then(|client| {
                    client.reachable[side]
                        .clone()
                        .map(|object| (object, None))
                        .or_else(|| {
                            client.grabbable[side]
                                .clone()
                                .map(|(object, distance)| (object, Some(distance)))
                        })
                });
            let rung = precedence::Grab::first(|rung| match rung {
                precedence::Grab::Transcript => side == Side::Right && wrist_active,
                // What the other hand holds, this hand's ray on it or the hand touching it, is the carrying's too: the
                // squeeze takes hold of it with both hands.
                precedence::Grab::Carried => {
                    self.projector.carrying_in(side)
                        || self.projector.joins(side)
                        || self.board.held_in(side).is_some()
                }
                precedence::Grab::Tomori => self.hands[side].aim == precedence::Aim::Tomori,
                precedence::Grab::Environment => target.is_some(),
                precedence::Grab::Nothing => true,
            });
            #[cfg(test)]
            {
                self.last_grab = Some((side, rung));
            }
            self.hands[side].gripping = match (rung, target) {
                (precedence::Grab::Environment, Some((object, distance))) => {
                    Gripping::Environment {
                        object,
                        distance,
                        told: false,
                        hand: None,
                    }
                }
                _ => Gripping::Other,
            };
        }
        let environment = matches!(self.hands[side].gripping, Gripping::Environment { .. });
        if !closed && !environment {
            self.hands[side].gripping = Gripping::Free;
        }
        environment
    }

    /// Drop a teleport being aimed, as when the left stick is lost, without travelling.
    pub fn cancel_travel(&mut self) {
        self.travel.cancel();
        self.travel_stick = Vec2::ZERO;
    }

    /// Drop a teleport being aimed with `side`'s stick, as when that stick is lost, without travelling; one aimed with
    /// the other stick goes on.
    pub fn cancel_travel_of(&mut self, side: Side) {
        if self.travel_right == (side == Side::Right) {
            self.cancel_travel();
        }
    }

    /// Where the participant's room is in the space's scene, in the space `conversation` is in: the transform the frame's poses are given in, which each platform applies to its head, eyes, controllers and hands before [`Self::update`]. Anywhere else, or in another space, the room is back home. See `design/locomotion.md`.
    pub fn rig(&mut self, conversation: &Conversation) -> Rig {
        self.placed = self
            .travel
            .settle(place(conversation), &conversation.locomotion());
        self.placed
    }

    /// The participant recentred: the floating panel stands in front of them again.
    pub fn recentre(&mut self) {
        self.panel.reset();
    }

    /// Whether the left stick rested at its middle as of the last frame's buttons, no push of it under way, so that
    /// a click of it is not part of travel. See `design/live-audio.md`.
    pub fn left_stick_free(&self) -> bool {
        self.left_stick == Push::Free
    }

    /// Whether the hand menu is pinned open with Y.
    pub fn pinned(&self) -> bool {
        self.hand_menu.pinned()
    }

    /// Let go of a menu pinned with Y. Returns whether it was pinned.
    #[cfg(not(target_os = "android"))]
    pub fn unpin(&mut self) -> bool {
        self.hand_menu.unpin()
    }

    /// Whether the right hand points this frame, from `trigger` and what the ray met: the palm turns while pointing across the body.
    pub fn pointing(&self, trigger: Option<f32>) -> bool {
        trigger.is_some_and(|trigger| trigger > POINTING)
            || matches!(self.hands.right.highlighted, Some(arrival::Key::Menu(_)))
    }

    /// The hand whose ray a button or stick of `own`'s controller acts on: its own, while its ray is on something, or
    /// else the other's, while that one's is; otherwise its own. So A ticks the item the right ray is on, or else the
    /// one the left ray is on. See `design/interaction.md`.
    fn serving(&self, own: Side) -> Side {
        let on = |side: Side| self.hands[side].aim != precedence::Aim::Nothing;
        if !on(own) && on(own.other()) {
            own.other()
        } else {
            own
        }
    }

    /// Act on this frame's `buttons` at `now`, `focused` if the participant has input. Buttons act on every frame, even one that is not drawn.
    pub fn buttons(
        &mut self,
        buttons: &Buttons,
        focused: bool,
        conversation: &mut Conversation,
        feedback: &mut Feedback,
        now: Instant,
    ) {
        let stick = buttons.left_stick;
        // Whose each push of a stick is, decided as it leaves the middle, until it returns there: the first rung of
        // `precedence` that claims it, Tomori's own interface first, so that the environment's client side has it only
        // with the ray on something of its own or on nothing of Tomori's. Each stick acts on what its own hand carries
        // and its own hand's ray is on, as the frame before found it, and the sticks alike: what the hand carries, what
        // its ray, shown in full, is on that takes the stick, and otherwise, where the environment allows it, travel,
        // so that a hand hanging at the side as the participant walks past a long panel captures no push. See
        // `design/interaction.md`.
        let environment = self.client.is_some();
        // A and B act on what the right ray is on, or else the left's.
        let serving = self.serving(Side::Right);
        let aim = self.hands[serving].aim;
        let travels = !matches!(conversation.locomotion(), locomotion::Locomotion::None);
        // While the right wrist's transcript shows, the right stick, A and B act on it, and B backs out of it before the space. See `design/wrist-transcript.md`.
        let wrist_active = self.wrist.state().active();
        // A push is the panel or wall its hand carries, if one is carried as it leaves the middle, until it returns there,
        // and then nothing else's, so that the transcript, which does not show while a panel is carried, or the cards,
        // never take a push that moved the panel, nor the panel one that moved them. Otherwise, a push that leaves the
        // middle with its ray on a panel with more than it shows scrolls it, for this participant alone, and nothing
        // else, while the right wrist's transcript does not take the right stick. See `design/projection.md`.
        let carrying = self.projector.carrying();
        let carries = Hands::from_fn(|side| self.projector.carrying_in(side));
        let scrolls = Hands::from_fn(|side| {
            let hand = &self.hands[side];
            hand.panel.clone().filter(|panel| {
                hand.full
                    && !(side == Side::Right && wrist_active)
                    && self.projector.scrollable(panel)
            })
        });
        // An object of the environment's that takes the stick, its hand's ray shown in full on it, has a push that nothing
        // of Tomori's takes, with its identity. See `design/environments.md`.
        let objects = Hands::from_fn(|side| {
            let hand = &self.hands[side];
            self.client
                .as_ref()
                .and_then(|client| client.scrollable[side].clone())
                .filter(|_| hand.full && hand.aim == precedence::Aim::Environment)
        });
        let on_tomori = |side: Side| {
            matches!(
                self.hands[side].aim,
                precedence::Aim::Tomori | precedence::Aim::Pad
            )
        };
        let left_rung = precedence::Left::first(|rung| match rung {
            precedence::Left::Carried => carries.left,
            precedence::Left::Scroll => scrolls.left.is_some(),
            precedence::Left::Object => objects.left.is_some(),
            precedence::Left::Travel => travels,
            precedence::Left::Tomori => on_tomori(Side::Left),
            precedence::Left::Environment => environment,
            precedence::Left::Nothing => true,
        });
        let rung = precedence::Right::first(|rung| match rung {
            precedence::Right::Carried => carries.right,
            precedence::Right::Transcript => wrist_active,
            precedence::Right::Scroll => scrolls.right.is_some(),
            precedence::Right::Object => objects.right.is_some(),
            precedence::Right::Travel => travels,
            precedence::Right::Tomori => on_tomori(Side::Right),
            precedence::Right::Environment => environment,
            precedence::Right::Cards => true,
        });
        // What a push takes as it leaves the middle, given what the other stick's push has under way: one push at a time
        // travels, scrolls or pushes an object, so that one begun while the other stick's does is nobody's.
        let claim = |shared: Option<SharedRung>, other: &Push, number: u64| match shared {
            Some(SharedRung::Carried) => Push::Carried,
            Some(SharedRung::Scroll(panel)) => match other {
                Push::Scrolled(_) => Push::Other,
                _ => Push::Scrolled(panel),
            },
            // A push begun without focus is nobody's, so that it never begins an object's.
            Some(SharedRung::Object(object)) => match other {
                Push::Object(..) => Push::Abandoned,
                _ if focused => Push::Object(object, number),
                _ => Push::Abandoned,
            },
            Some(SharedRung::Travel) => match other {
                Push::Travel => Push::Other,
                _ => Push::Travel,
            },
            Some(SharedRung::Environment) => Push::Environment,
            None => Push::Other,
        };
        let left_shared = match left_rung {
            precedence::Left::Carried => Some(SharedRung::Carried),
            precedence::Left::Scroll => scrolls.left.clone().map(SharedRung::Scroll),
            precedence::Left::Object => objects.left.clone().map(SharedRung::Object),
            precedence::Left::Travel => Some(SharedRung::Travel),
            precedence::Left::Environment => Some(SharedRung::Environment),
            precedence::Left::Tomori | precedence::Left::Nothing => None,
        };
        let right_shared = match rung {
            precedence::Right::Carried => Some(SharedRung::Carried),
            precedence::Right::Scroll => scrolls.right.clone().map(SharedRung::Scroll),
            precedence::Right::Object => objects.right.clone().map(SharedRung::Object),
            precedence::Right::Travel => Some(SharedRung::Travel),
            precedence::Right::Environment => Some(SharedRung::Environment),
            precedence::Right::Transcript
            | precedence::Right::Tomori
            | precedence::Right::Cards => None,
        };
        // The left stick's push is decided first, so that of two begun together, the left's travels.
        let number = self.pushes + 1;
        let right_before = self.right_stick.clone();
        self.left_stick = std::mem::take(&mut self.left_stick)
            .update(stick, || claim(left_shared, &right_before, number));
        if let Push::Object(_, number) = &self.left_stick {
            self.pushes = self.pushes.max(*number);
            self.wheel_push = None;
        }
        let number = self.pushes + 1;
        let left_now = self.left_stick.clone();
        self.right_stick = std::mem::take(&mut self.right_stick)
            .update(buttons.right_stick, || {
                claim(right_shared, &left_now, number)
            });
        if let Push::Object(_, number) = &self.right_stick {
            self.pushes = self.pushes.max(*number);
            self.wheel_push = None;
        }
        // Outside a space, the left stick moves in what the lobby lists, whoever's its push is.
        let moving = stick.x.abs() > STICK || stick.y.abs() > STICK;
        let navigate = moving && !self.stick_latched;
        self.stick_latched = moving;
        // In a space the left stick is free: the panel it paged is now the transcript on the right wrist.
        if navigate && conversation.joined_space().is_none() {
            if stick.y.abs() >= stick.x.abs() {
                conversation.navigate(0, if stick.y > 0.0 { -1 } else { 1 });
            } else {
                conversation.navigate(if stick.x > 0.0 { 1 } else { -1 }, 0);
            }
        }
        let of = |push: &Push, value: Vec2, owner: fn(&Push) -> bool| {
            if focused && owner(push) {
                value
            } else {
                Vec2::ZERO
            }
        };
        let (left, right) = (&self.left_stick, &self.right_stick);
        let carried = |push: &Push| *push == Push::Carried;
        let travelling = |push: &Push| *push == Push::Travel;
        let environmental = |push: &Push| *push == Push::Environment;
        let pushing = |push: &Push| matches!(push, Push::Object(..));
        self.carry_stick = Hands {
            left: of(left, stick, carried),
            right: of(right, buttons.right_stick, carried),
        };
        // In a space whose environment allows it, the stick whose push travels does, which the frame step does with its
        // hand's ray; see `design/locomotion.md`.
        self.travel_stick =
            of(left, stick, travelling) + of(right, buttons.right_stick, travelling);
        if travelling(left) {
            self.travel_right = false;
        } else if travelling(right) {
            self.travel_right = true;
        }
        let environment_left = of(left, stick, environmental);
        let environment_right = of(right, buttons.right_stick, environmental);
        // The right stick moves the transcript's focus or pages cards with what nothing else took.
        let right = if matches!(self.right_stick, Push::Free | Push::Other) {
            buttons.right_stick
        } else {
            Vec2::ZERO
        };
        self.scroll_axis = if pushing(&self.left_stick) {
            stick
        } else if pushing(&self.right_stick) {
            buttons.right_stick
        } else {
            Vec2::ZERO
        };
        self.scroll_stick = match (&self.left_stick, &self.right_stick) {
            (Push::Scrolled(panel), _) if focused => Some((panel.clone(), stick.y)),
            (_, Push::Scrolled(panel)) if focused => Some((panel.clone(), buttons.right_stick.y)),
            _ => None,
        };
        self.environment_sticks = (environment_left, environment_right);
        let object = objects.right.as_deref();
        // The mouse's wheel scrolls the panel pointed at, unless one is carried or the transcript shows.
        if let Some(panel) = &self.hands[serving].panel
            && focused
            && !carrying
            && !wrist_active
            && buttons.scroll.is_finite()
            && buttons.scroll != 0.0
        {
            match &mut self.wheel {
                Some((scrolled, rows)) if scrolled == panel => *rows += buttons.scroll,
                wheel => *wheel = Some((panel.clone(), buttons.scroll)),
            }
        }
        // Pointing at an object that takes the right stick, where the stick's push would be its own, a turn of the wheel
        // is a push of the stick, forward for a turn away from the participant, that scrolls as a panel would the rows
        // the wheel turned, as fast as the stick scrolls one at most, and goes on while it is turned. See
        // `design/desktop.md`.
        if let Some(object) = object
            && rung == precedence::Right::Object
            && focused
            && self.right_stick == Push::Free
            && !matches!(self.left_stick, Push::Object(..))
            && buttons.scroll.is_finite()
            && buttons.scroll != 0.0
        {
            let wheel = match self.wheel_push.take() {
                Some(wheel) if wheel.object == object => wheel,
                _ => {
                    self.pushes += 1;
                    WheelPush::new(object, self.pushes)
                }
            };
            self.wheel_push.insert(wheel).turn(buttons.scroll);
        }
        if !focused {
            self.abandon_scroll();
        }
        // B pressed while a panel is carried is the panel's, and nothing else's until it is let go: let go soon enough,
        // while the panel is still carried, it throws the panel away for everyone; held longer, it does nothing, and
        // never leaves the space. B on its own never dismisses a panel. See `design/projection.md`.
        let thrown = buttons.back && carrying;
        // Whether what B was pressed on was a panel, before it is thrown away.
        let panel_thrown = thrown && self.projector.held().is_some();
        if thrown {
            self.throwing = self.projector.held().map(|panel| (panel.to_owned(), now));
            self.removing = self
                .projector
                .carried_wall()
                .map(|wall| (wall.to_owned(), now));
        }
        // B pressed while carrying a wall is the wall's, as for a panel: let go soon enough, while it is still carried, it
        // removes the wall, for an owner, asking again first when it holds panels. See `design/projection.md`.
        if let Some((wall, since)) = self.removing.take_if(|_| !buttons.back_held) {
            let quick = now.saturating_duration_since(since) < crate::conversation::DISMISS_PRESS;
            if quick
                && self.projector.carried_wall() == Some(wall.as_str())
                && conversation.wall_rights().remove
                && let Some(projection) = conversation.projection().cloned()
            {
                match self.projector.remove_wall(&projection, now) {
                    Some(wall) => {
                        conversation.remove_wall_by_id(wall);
                        feedback.emit(feedback::Event::Choose, now);
                    }
                    None => feedback.emit(feedback::Event::Hover, now),
                }
            }
        }
        // B with a wall taken hold of, and nothing carried, lets go of it, before backing out of anything else.
        let released_wall = buttons.back && !thrown && self.projector.stop_editing();
        if let Some((panel, since)) = self.throwing.take_if(|_| !buttons.back_held) {
            let quick = now.saturating_duration_since(since) < crate::conversation::DISMISS_PRESS;
            if quick
                && self.projector.held() == Some(panel.as_str())
                && conversation.dismiss_panel(&panel)
            {
                self.projector.throw(now);
                feedback.emit(feedback::Event::Choose, now);
            }
        }
        let back = buttons.back && !thrown && !released_wall;
        // A press on a wall held, not carried, still starts holding to leave, as every rung but what is carried does.
        let leaving = buttons.back && !thrown;
        // B lets go of a menu pinned with Y first.
        let unpinned = back && self.hand_menu.unpin();
        let wrist_input = self.wrist.input(
            conversation,
            right,
            buttons.a && !self.menu_was_shown,
            back && !unpinned,
            now,
        );
        // Moving the transcript's focus onto a turn, a request's button or a queued entry ticks as hover does, and A choosing taps; see `design/feedback.md`.
        if wrist_input.moved {
            feedback.emit(feedback::Event::Hover, now);
        }
        if wrist_input.on_request {
            feedback.request_opened();
        }
        if wrist_input.chose {
            feedback.emit(feedback::Event::Choose, now);
        }
        // Whose B is, in the order `conversation::Back` sets out: the first three here, which the conversation does not
        // hold, and the rest in the conversation, which ends with the environment's client side, while the ray points at
        // a selectable object of its. It still starts holding to leave, below. See `design/interaction.md`.
        let taken = if !buttons.back {
            None
        } else if panel_thrown {
            Some(crate::conversation::Back::Carried)
        } else if thrown {
            Some(crate::conversation::Back::CarriedWall)
        } else if released_wall {
            Some(crate::conversation::Back::HeldWall)
        } else if unpinned {
            Some(crate::conversation::Back::Pinned)
        } else if wrist_input.back {
            Some(crate::conversation::Back::Wrist)
        } else {
            let focus = self
                .client
                .as_ref()
                .is_some_and(|client| client.focus.is_some());
            Some(conversation.back_or_environment(focus))
        };
        if taken == Some(crate::conversation::Back::Environment)
            && let Some(client) = self.client.as_mut()
        {
            client.back = true;
        }
        #[cfg(test)]
        if taken.is_some() {
            self.last_back = taken;
        }
        // Only a hold whose press is in a space, and not on a panel carried, can leave it, so B held while joining does
        // not.
        if leaving && conversation.joined_space().is_some() {
            self.back_since = Some(now);
        }
        match self.back_since {
            // A hold ends with the space it began in, so that it never leaves the next one.
            Some(since) if buttons.back_held && conversation.joined_space().is_some() => {
                let held = now.duration_since(since);
                feedback.hold_to_leave(Some(
                    held.as_secs_f32() / crate::conversation::LEAVE_HOLD.as_secs_f32(),
                ));
                if conversation.hold_back(Some(held)) {
                    feedback.emit(feedback::Event::Left, now);
                }
            }
            Some(_) => {
                self.back_since = None;
                conversation.hold_back(None);
            }
            None => {}
        }
        if self.back_since.is_none() {
            feedback.hold_to_leave(None);
        }
        conversation.tick_space(now);
        // Y in a space pins the hand menu open at its Space section, until Y or B; elsewhere it browses or lists again. See `design/hand-menu.md`.
        if conversation.joined_space().is_none() {
            self.hand_menu.unpin();
        }
        if buttons.y {
            if conversation.joined_space().is_some() {
                self.hand_menu.toggle_pin();
                if self.hand_menu.pinned() {
                    feedback.emit(feedback::Event::Shown(feedback::Hand::Left), now);
                }
            } else {
                conversation.refresh();
            }
        }
        // Whose a press of A is, in the order of `precedence::A`, the one place that orders them: a hand menu row the ray
        // is on, which it chooses, as the trigger does; anything else while the hand menu shows, where it does nothing,
        // and while the transcript shows, only that; the conversation's, which accepts, or in a space chooses from the
        // list, ticks the item pointed at, decides or cancels, and never resets the swarm there; anything else of
        // Tomori's the ray is on, where it does nothing; and last, a selectable object of the environment's client side
        // that the ray pointed at last frame, which it selects. See `design/interaction.md`.
        let focus = self.client.as_ref().and_then(|client| client.focus.clone());
        let mut chose = false;
        // The menu row or arrival object the right ray is on, or else the left's.
        let highlighted = self.hands[Side::Right]
            .highlighted
            .clone()
            .or_else(|| self.hands[Side::Left].highlighted.clone());
        let rung = buttons.a.then(|| {
            precedence::A::first(|rung| match rung {
                precedence::A::MenuRow => matches!(highlighted, Some(arrival::Key::Menu(_))),
                precedence::A::Menu => self.menu_was_shown,
                precedence::A::Transcript => wrist_active,
                precedence::A::Conversation => {
                    let nothing = conversation.press_a(highlighted.as_ref());
                    chose = !nothing && !conversation.a_refused();
                    !nothing
                }
                precedence::A::Tomori => aim == precedence::Aim::Tomori,
                precedence::A::Environment => focused && focus.is_some(),
                precedence::A::Nothing => true,
            })
        });
        match (rung, &highlighted) {
            // The ray only meets rows that can be chosen.
            (Some(precedence::A::MenuRow), Some(arrival::Key::Menu(key))) => {
                feedback.emit(feedback::Event::Choose, now);
                if let Some(choice) = self.hand_menu.choose(key) {
                    conversation.choose_menu(choice);
                }
            }
            (Some(precedence::A::Conversation), _) if chose => {
                feedback.emit(feedback::Event::Choose, now);
            }
            _ => {}
        }
        // A selects the object the environment's client side had in focus, as the trigger would, and is let go with A,
        // wherever the ray is then, or on losing focus. It is told with the trigger's pulls, in `update`, which does
        // not run on every frame the buttons are read, as on the headset when nothing is drawn: a press waits there,
        // the first of any since, until `update` takes it, as a press of B does, and what is held is as the latest
        // buttons found it, so that a press and its release across such frames are still pressed and let go once.
        self.a_press = crate::client_side::Press {
            on: self
                .a_press
                .on
                .take()
                .or_else(|| focus.filter(|_| rung == Some(precedence::A::Environment)))
                .filter(|_| focused),
            held: focused && buttons.a_held,
        };
        // Hold X to talk to the agent; releasing it while focused sends the utterance.
        if focused && !buttons.talk {
            self.talk_blocked = false;
        }
        let talking = focused && buttons.talk && !self.talk_blocked;
        if talking != self.was_talking {
            conversation.talk(talking);
        }
        self.was_talking = talking;
        // The desktop's V mutes spoken replies locally; right stick click plays the latest reply again, or stops it.
        if buttons.mute {
            conversation.toggle_mute();
        }
        // While a panel is carried, the click is its: it turns back to face the participant square.
        if buttons.replay && carrying {
            self.straighten = true;
        } else if buttons.replay {
            conversation.replay();
        }
        // Otherwise the right stick pages result cards.
        self.presentation_navigation.update(
            None,
            &delegation::Workspace::default(),
            if wrist_active { 0.0 } else { right.x },
            if wrist_active { 0.0 } else { right.y },
        );
    }

    /// Read the system's clock, in seconds since the Unix epoch, from `system` rather than the system, as a test does to
    /// make it jump. See `design/timers.md`.
    pub fn set_clock(&mut self, system: impl Fn() -> f64 + Send + Sync + 'static) {
        self.system = Some(Box::new(system));
    }

    /// The time at `now`, in seconds since the Unix epoch, by this client's clock.
    fn unix(&mut self, now: Instant) -> f64 {
        let system = match &self.system {
            Some(system) => system(),
            None => std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs_f64(),
        };
        self.clock.now(system, now)
    }

    /// Move everything on by `dt` seconds to `now`, as `senses` find it, and say what to draw and report.
    pub fn update(
        &mut self,
        senses: &Senses,
        conversation: &mut Conversation,
        feedback: &mut Feedback,
        now: Instant,
        dt: f32,
    ) -> Frame<'_> {
        let focused = senses.focused;
        let visual = self.signals.update(
            interaction::Inputs {
                session: if focused {
                    conversation.activity()
                } else {
                    interaction::SessionActivity::Unknown
                },
                speech: conversation.speech(),
                ..Default::default()
            },
            dt,
        );
        // Follows the agent's activity, which can change many times a minute.
        if visual.mode != self.coordinator.visual().mode {
            log::debug!(
                "Swarm interaction: {:?}, session_working={}",
                visual.mode,
                visual.working
            );
        }
        // Each hand's ray, trigger and grip, which act alike on Tomori's own things. See `design/interaction.md`.
        let rays = Hands {
            right: senses.ray,
            left: senses.left_ray,
        };
        let triggers = Hands {
            right: senses.trigger,
            left: senses.left_trigger,
        };
        let grips = Hands {
            right: senses.grip,
            left: senses.left_grip,
        };
        // Arrival's objects move between its steps rather than cutting, and are pointed at where they are now. See `design/arrival.md`.
        self.arrival_time += dt;
        self.motion
            .update(&arrival_target(conversation), self.arrival_time);
        let mut scene = self.motion.scene(self.arrival_time);
        let moment = Moment {
            now,
            dt,
            time: self.arrival_time,
        };
        // While recording, the indicator rises from the left controller, or the left wrist, or else stands low in the view. See `design/hand-menu.md`.
        let indicator = self
            .indicator
            .update(conversation, senses.trackers, senses.viewer, moment);
        // The hand menu shows above the left wrist while the left palm is turned up, as the right palm shows the transcript. Whichever of it and the right transcript shows first wins; while it shows, the trigger chooses only in it. See `design/hand-menu.md`.
        let menu_shown = focused
            && self.hand_menu.update(
                senses.left_palm.filter(|_| !self.wrist.state().on_hand()),
                now,
            );
        self.menu_was_shown = menu_shown;
        // Shown on the left palm, rather than pinned with Y, the menu is the right ray's alone, and the left hand,
        // holding it, points at nothing and chooses nothing.
        let menu_on_palm = menu_shown && !self.hand_menu.pinned();
        let in_space = conversation.joined_space().is_some();
        // Whose each grip's squeeze is, decided as it closes, until it is let go: Tomori's own first, and the
        // environment's object only where none of them takes it, which nothing of Tomori's is then given. See
        // `design/interaction.md` and `design/environments.md`.
        let wrist_was_active = self.wrist.state().active();
        let participates = conversation.role().participates();
        let tomori_grips = Hands::from_fn(|side| {
            let environment = self.grip(
                side,
                grips[side].filter(|_| focused && in_space),
                wrist_was_active,
                participates,
            );
            if environment { Some(0.0) } else { grips[side] }
        });
        let (wrist_opened, wrist) = self.wrist.update(
            conversation,
            WristFrame {
                palm: senses.right_palm.filter(|_| focused),
                hands_only: senses.hands_only,
                head: senses.head,
                viewer: senses.viewer,
                in_space,
                menu_shown,
                menu_pinned: self.hand_menu.pinned(),
                // A panel carried in the right hand keeps the transcript away, as pointing does, so that turning the
                // wrist does not let it go.
                pointing: senses.pointing || self.projector.carrying_in(Side::Right),
                grip: tomori_grips.right,
            },
            moment,
        );
        // The transcript appearing on the wrist ticks very lightly, and counts as looking at a request it shows.
        if wrist_opened {
            feedback.request_opened();
            feedback.emit(feedback::Event::Shown(feedback::Hand::Right), now);
        }
        let wrist_shown = self.wrist.state().on_hand();
        let wrist_active = self.wrist.state().active();
        let wrist_waiting = conversation.wrist_waiting();
        // What the rays pointed at last frame, so that the description of the mode pointed at shows beneath the modes, in rows that are there whatever is pointed at, so that the rows do not move under the ray.
        let pointed = Side::BOTH
            .into_iter()
            .find_map(|side| match &self.hands[side].highlighted {
                Some(arrival::Key::Menu(key)) => Some(key.clone()),
                _ => None,
            });
        let menu = |hand_menu: &HandMenu, conversation: &Conversation| {
            let mut objects = match senses.left_palm {
                Some(palm) if menu_shown => {
                    let mut model = conversation.menu_model();
                    model.pointed = pointed.clone();
                    hand_menu.objects(&palm, &model, senses.viewer.unwrap_or(arrival::HOME))
                }
                Some(palm) if senses.cues && conversation.menu_model().waiting() => {
                    vec![hand_menu::cue(&palm)]
                }
                _ => Vec::new(),
            };
            // A cue on the right wrist while the transcript is closed and something waits on it.
            if let (true, Some(palm), Some(problem), false) =
                (senses.cues, senses.right_palm, wrist_waiting, wrist_active)
            {
                objects.push(hand_menu::wrist_cue(&palm, problem));
            }
            objects
        };
        scene.objects.extend(menu(&self.hand_menu, conversation));
        // Labels and forms face the participant's head, in steps of 5 cm so that they are not rebuilt every frame as the head sways.
        let viewer = senses.viewer.map(|head| (head * 20.0).round() / 20.0);
        scene.viewer = viewer;
        let targets: Vec<_> = scene
            .targets()
            .into_iter()
            .filter(|target| !menu_shown || matches!(target.key, arrival::Key::Menu(_)))
            .collect();
        // Each ray points at arrival's objects and the menu's rows, and its trigger chooses them; the left, holding the
        // menu on its palm, at none.
        let mut pointings = Hands::from_fn(|side| {
            let targets = if side == Side::Left && menu_on_palm {
                &[][..]
            } else {
                &targets[..]
            };
            self.hands[side]
                .pointer
                .update(rays[side], triggers[side], targets)
        });
        let mut chose = false;
        for side in Side::BOTH {
            self.hands[side].highlighted = pointings[side].hit.as_ref().map(|hit| hit.key.clone());
            if let Some(key) = pointings[side].chosen.take() {
                chose = true;
                feedback.emit_on(feedback::Event::Choose, side.into(), now);
                match key {
                    arrival::Key::Menu(key) => {
                        if let Some(choice) = self.hand_menu.choose(&key) {
                            conversation.choose_menu(choice);
                        }
                    }
                    key => conversation.choose_object(&key),
                }
            }
        }
        if chose {
            self.motion
                .update(&arrival_target(conversation), self.arrival_time);
            scene = self.motion.scene(self.arrival_time);
            scene.objects.extend(menu(&self.hand_menu, conversation));
            scene.viewer = viewer;
        }
        let in_arrival = conversation.arrival().is_some();
        let in_space_shown = !in_arrival && conversation.joined_space().is_some();
        // Arrival and the lobby show the grid, and a space its own environment, growing out of the space's object as it is entered; no control switches it. See `design/environments.md`.
        let scenery = self.motion.scenery(self.arrival_time);
        // Whose each pull of a trigger is, decided as it begins, until it is let go: the first rung of
        // `precedence::Trigger` that claims it for that hand, as its ray found things last frame, Tomori's own interface
        // first. A pull that begins on a selectable object of the environment's client side, with nothing of Tomori's
        // taking it, is the client side's, and does not attract the swarm: for now, the right trigger's alone. See
        // `design/interaction.md` and `design/environments.md`.
        let focus = self
            .client
            .as_ref()
            .and_then(|client| client.focus.as_deref());
        // A choosable row of an environment's list each ray met last frame, for a participant: a pull begun on it
        // chooses it, as A does, rather than attracting the swarm. See `design/panel-actions.md`.
        let rows = Hands::from_fn(|side| {
            self.hands[side]
                .item
                .as_ref()
                .and_then(|item| conversation.choosable_row(item))
                .filter(|_| conversation.ticks())
        });
        let rungs = Hands::from_fn(|side| {
            let aim = self.hands[side].aim;
            precedence::Trigger::first(|rung| match rung {
                precedence::Trigger::Menu => menu_shown || in_arrival,
                precedence::Trigger::Transcript => side == Side::Right && wrist_active,
                precedence::Trigger::CarriedWall => self.projector.carried_wall_in(side).is_some(),
                precedence::Trigger::Row => aim == precedence::Aim::Tomori && rows[side].is_some(),
                precedence::Trigger::Tomori => aim == precedence::Aim::Tomori,
                precedence::Trigger::Pad => aim == precedence::Aim::Pad,
                precedence::Trigger::Environment => side == Side::Right && focus.is_some(),
                precedence::Trigger::Swarm => true,
            })
        });
        // One select at a time, across both hands: a pull that begins while another is held is nobody's.
        let holding = |step: &Self, but: (Side, u8)| {
            (step.selector.holds() && but != (Side::Right, 0))
                || Side::BOTH.into_iter().any(|side| {
                    (step.hands[side].pad_selector.holds() && but != (side, 1))
                        || (step.hands[side].row_selector.holds() && but != (side, 2))
                })
        };
        let others = holding(self, (Side::Right, 0));
        let select = self.selector.update_beside(
            focus.filter(|_| rungs.right == precedence::Trigger::Environment),
            senses.trigger,
            // A press is told once; A held is as the last buttons found it, until the next. A press in the frame the hand
            // menu or the transcript appears is theirs, as a pull of the trigger is, though A's own rungs, which are
            // decided before this frame finds them, have always gone by what showed the frame before.
            crate::client_side::Press {
                on: self
                    .a_press
                    .on
                    .take()
                    .filter(|_| !menu_shown && !wrist_active),
                held: self.a_press.held,
            },
            others,
        );
        if let Some(client) = self.client.as_mut() {
            client.select = client.select.take().or(select);
        }
        // A pull begun on a teleport pad chooses it once pressed as far as choosing an arrival object takes, as A does,
        // which the conversation's `accept` took for it; either takes the participant to the pad paired with it, from
        // this frame's travel. See `design/locomotion.md`.
        let mut pulled = None;
        for side in Side::BOTH {
            let pad_name = self.hands[side]
                .pad
                .filter(|_| rungs[side] == precedence::Trigger::Pad)
                .map(|way| match way {
                    crate::pads::Way::Pad(key) => format!("{}/{}", key.pair, key.end),
                    crate::pads::Way::Target(index) => format!("target/{index}"),
                });
            let others = holding(self, (side, 1));
            if matches!(
                self.hands[side].pad_selector.update_beside(
                    pad_name.as_deref(),
                    triggers[side],
                    crate::client_side::Press::default(),
                    others,
                ),
                Some(crate::client_side::Select::Pressed(_))
            ) {
                pulled = pulled.or(self.hands[side].pad);
            }
        }
        let pressed = conversation.take_pad_choice();
        let serving = self.serving(Side::Right);
        let chosen_pad = pulled.or(self.hands[serving].pad.filter(|_| pressed));
        // Pressed as far as choosing takes, on the row it began on, it chooses that row, with the choosing click.
        for side in Side::BOTH {
            let others = holding(self, (side, 2));
            if let Some(crate::client_side::Select::Pressed(_)) =
                self.hands[side].row_selector.update_beside(
                    rows[side]
                        .as_ref()
                        .map(|(panel, row)| format!("{panel}\n{row}"))
                        .as_deref()
                        .filter(|_| rungs[side] == precedence::Trigger::Row),
                    triggers[side],
                    crate::client_side::Press::default(),
                    others,
                )
                && let Some(row) = rows[side].clone()
                && conversation.choose_row_at(row)
            {
                feedback.emit_on(feedback::Event::Choose, side.into(), now);
            }
        }
        // A trigger pull that misses the hand menu's rows does nothing while it shows, nor while the transcript shows.
        // While a wall is carried, its hand's trigger keeps it from snapping rather than attracting the swarm. See
        // `design/projection.md`. In a space it draws the swarm nowhere: the agent comes to whoever speaks to it, and the
        // trigger is for pointing and selecting. Outside one, either trigger draws it towards its own hand, and a pull
        // of both towards the first. See `design/interaction.md`.
        let attracts = Hands::from_fn(|side| {
            let selecting = (side == Side::Right && self.selector.owns())
                || self.hands[side].pad_selector.owns()
                || self.hands[side].row_selector.owns();
            pointings[side].attracting
                && !in_space_shown
                && !menu_shown
                && !(side == Side::Right && wrist_active)
                && !selecting
                && rungs[side] != precedence::Trigger::CarriedWall
        });
        self.attracting = self
            .attracting
            .filter(|side| attracts[*side])
            .or(Side::BOTH.into_iter().find(|side| attracts[*side]));
        let attracting = self.attracting.is_some();
        let attractor = self.attracting.and_then(|side| match side {
            Side::Right => senses.attractor,
            Side::Left => senses.left_attractor,
        });
        // The agent appears at its home as a space opens, or changes environment, and otherwise glides to where it goes
        // and back home; the workers stand about its home. See `design/environments.md`.
        let arrangement = conversation.arrangement();
        // Leaving a space, the rig goes home and the space's state is cleared at once, while its environment still
        // shrinks back into the space's object: the environment stays where the participant saw it, in their room, until
        // it has gone, so that nothing jumps as the space is left. See `design/arrival.md`.
        let placed = self.placed;
        let space = conversation
            .joined_space()
            .map(|(space, _)| space.id.as_str());
        let in_a_space = space.is_some();
        // Joining a space, from the lobby or from another, and leaving one, the agent's place is where everyone sees it
        // at once, but this client's swarm flies there from where its participant saw it, through the environment
        // growing in or shrinking away, as a flight that only they see. See `design/arrival.md`.
        let joining = in_a_space && space != self.seen_space.as_deref();
        let leaving = !in_a_space && self.seen_space.is_some();
        // The rig moving for anything but travel within a space, as it goes home on leaving, moves the scene under the
        // participant: the swarm is carried with them, so that it flies on from where they saw it.
        if placed != self.seen_rig && !(in_a_space && !joining) {
            self.coordinator.carry(
                self.seen_rig.place(crate::anchors::Anchor::HOME),
                placed.place(crate::anchors::Anchor::HOME),
            );
        }
        self.seen_rig = placed;
        if self.seen_space.as_deref() != space {
            self.seen_space = space.map(str::to_owned);
        }
        let departing = !in_a_space
            && self
                .standing
                .is_some_and(|standing| standing.environment == scenery.environment);
        if !in_a_space && !departing {
            self.standing = None;
        }
        let departure = self.standing.filter(|_| departing).map(|standing| {
            // The rig going home moves the scene under the participant: the environment is carried with them.
            (
                placed.place(standing.carriage),
                carried_ring(standing.ring, |point| placed.place(at(point)).position),
            )
        });
        let carriage = departure.map_or(arrangement.carriage, |(carriage, _)| carriage);
        let flying = joining || leaving;
        let appear = flying || arrangement.home != self.agent_home;
        self.agent_home = arrangement.home;
        let anchor = arrangement.agent;
        // The ring on the floor stays at home, wherever the agent goes, and with the environment as it shrinks away.
        let home_ring = departure.map_or(arrangement.home_ring(), |(_, ring)| ring);
        // In a space whose environment is drawn, where it stands in the participant's room, for as long as it shrinks
        // away should the space be left.
        if in_a_space {
            self.standing =
                (scenery.environment != crate::environment::Environment::MINIMAL).then(|| {
                    Standing {
                        environment: scenery.environment,
                        carriage: placed.unplace(arrangement.carriage),
                        ring: carried_ring(home_ring, |point| placed.unplace(at(point)).position),
                    }
                });
        }
        // The agent stands there as the avatar the space's backend, or its environment, names, where this build
        // registers it, or the swarm. See `design/avatars.md`.
        self.coordinator.choose(conversation.avatar());
        self.coordinator.update(
            &crate::avatar::Input::new(visual, anchor, dt)
                .appearing(appear)
                .flying(flying)
                .speaking(conversation.speech().playing)
                .head(senses.head_pose.map(|pose| Vec3::from_array(pose.position)))
                .attracted(attractor)
                .going(arrangement.via, arrangement.stream)
                .near(arrangement.nearness()),
        );
        let marker = match senses.marker {
            Marker::Always => senses.attractor,
            Marker::WhileAttracting => attractor,
            Marker::Hidden => None,
        };
        // In a space each ray points at what the space shows, which only highlights it, its trigger and grip acting on it as the ladders say, and holding X sends what the rays point at with the words. See `design/interaction.md`. While the hand menu shows, the rays are its; while the transcript shows, the right ray is hidden and points at nothing, and the left goes on.
        let world = Hands::from_fn(|side| {
            focused
                && !in_arrival
                && !menu_shown
                && !(side == Side::Right && wrist_active)
                && conversation.joined_space().is_some()
        });
        // A ray whose hand points down, as an arm resting at the side does, is hidden, and points at nothing, unless its
        // hand is busy: holding something, squeezing or pulling. Only where the platform draws its rays: the desktop's
        // mouse points wherever it does.
        let busy = Hands::from_fn(|side| {
            triggers[side].is_some_and(|trigger| trigger > crate::pointing::PULL)
                || self.hands[side].grip_closed
                || self.projector.carrying_in(side)
                || self.board.held_in(side).is_some()
        });
        for side in Side::BOTH {
            let down = rays[side].map(|ray| -ray.direction.y);
            let hand = &mut self.hands[side];
            hand.lowered = senses.beam
                && down.is_some_and(|down| {
                    if hand.lowered {
                        down > RAISED
                    } else {
                        down > LOWERED
                    }
                });
        }
        let casts =
            Hands::from_fn(|side| rays[side].filter(|_| !self.hands[side].lowered || busy[side]));
        // In an environment that allows it, the left stick travels: aiming a teleport with the left ray, the hand whose stick aims it, landing as it returns to the middle, turning, stepping back and rising, each push owned by the direction it went first. The rig it moves applies from the next frame, as the platform places this frame's poses with it. See `design/locomotion.md`.
        let locomotion = conversation.locomotion();
        // The hand whose ray aims: the left on the headset, whose stick aims, and the mouse on the desktop, or the right
        // while the right stick's push travels, until it has been followed to its end.
        let aimer = if self.travel_right {
            Side::Right
        } else {
            senses.teleport
        };
        // Only once the environment's scenery has grown in, so that its surfaces are seen; not while the transcript shows; and not while the left hand carries a card, since aiming would put it down. A panel carried in either hand travels along, in the hand, as the rig carries the ray it follows.
        let grown = scenery.reveal.is_none() && scenery.environment == conversation.environment();
        let travel_active =
            world[aimer] && !wrist_active && grown && self.board.held_in(aimer).is_none();
        let travel = self.travel.update(locomotion::Input {
            place: place(conversation),
            locomotion: &locomotion,
            stick: self.travel_stick,
            ray: rays[aimer],
            head: senses.head_pose.map(|pose| Vec3::from_array(pose.position)),
            facing: senses
                .head_pose
                .and_then(|pose| locomotion::facing(glam::Quat::from_array(pose.orientation))),
            active: travel_active,
            chosen: chosen_pad,
            dt,
        });
        if self.right_stick != Push::Travel {
            self.travel_right = false;
        }
        // Landing clicks as choosing does, on the hand that aimed; a pad chosen with A clicked as A chose it.
        if travel.teleported || (travel.transported && !pressed) {
            feedback.emit_on(feedback::Event::Choose, aimer.into(), now);
        }
        // While a teleport is aimed, the left ray serves only it, though a panel carried in either hand stays there; the
        // right ray goes on pointing, choosing and carrying.
        let carrying_through = world;
        let points = Hands::from_fn(|side| world[side] && !(side == aimer && travel.aiming));
        // What is projected stands wherever the space shows it; the rays and grips act on it only while nothing else takes them, and not while that hand carries a card. A card a ray met nearer last frame takes that hand's grip. See `design/projection.md`.
        conversation.observe_head(senses.head_pose.map(Into::into));
        // A change to a wall the server refused is drawn as the server has it at once, not as sent, and so is a panel's
        // size.
        if let Some(wall) = conversation.take_refused_wall() {
            self.projector.refused_wall(&wall);
        }
        if let Some(panel) = conversation.take_refused_panel() {
            self.projector.refused_panel(&panel);
        }
        // The teleport pad each ray meets, if any, which nothing nearer takes, below; a panel behind it is not pulled
        // off, as one behind a card is not. A ray pointing down still meets the pads and targets on the floor, as the
        // pad stood on, and shows while it does.
        let pads_met = Hands::from_fn(|side| {
            rays[side]
                .filter(|_| {
                    points[side]
                        && grown
                        && self.board.held_in(side).is_none()
                        && !self.projector.carrying_in(side)
                })
                .and_then(|ray| crate::pads::point(&locomotion, &ray))
        });
        // A list's choosable row pointed at last frame is marked where A or the trigger would choose it for this
        // participant, and the row just chosen for as long as the label says so.
        self.projector.mark_rows(crate::projection::RowMarks {
            hovered: conversation.pointed_row().filter(|_| conversation.ticks()),
            chosen: conversation
                .chosen_row()
                .map(|(panel, row)| (panel.to_owned(), row.to_owned())),
        });
        let hold = |step: &Self, side: Side| crate::projection::Hold {
            ray: casts[side],
            // Only a participant pulls panels off; a visitor only looks at them and points.
            grip: tomori_grips[side].filter(|_| step.board.held_in(side).is_none() && participates),
            active: carrying_through[side] && step.board.held_in(side).is_none(),
            pointing: points[side],
            nearer: pads_met[side].map_or(step.hands[side].card_reach, |(_, distance, _)| {
                step.hands[side].card_reach.min(distance)
            }),
            free: triggers[side].is_some_and(|trigger| trigger > crate::pointing::PULL),
            stick: step.carry_stick[side],
        };
        let right = hold(self, Side::Right);
        let left = hold(self, Side::Left);
        // Which hands held a panel or a card before this frame, so that what each takes or puts down is felt on it.
        let held_before = Hands::from_fn(|side| self.projector.held_in(side).is_some());
        // The environment's floors and decks hide a locked wall's padlock beyond them, and stop the ray before what is.
        let surfaces = conversation.surfaces();
        let projecting = Instant::now();
        let projected = self.projector.update(crate::projection::Input {
            projection: conversation.projection().filter(|_| !in_arrival),
            ray: right.ray,
            grip: right.grip,
            viewer: senses.viewer,
            active: right.active,
            pointing: right.pointing,
            nearer: right.nearer,
            stick: right.stick,
            straighten: std::mem::take(&mut self.straighten),
            free: right.free,
            controls: self.controls,
            scroll: scrolled(self.scroll_stick.as_ref(), self.wheel.take(), dt)
                .as_ref()
                .map(|(panel, rows)| (panel.as_str(), *rows)),
            rights: conversation.wall_rights(),
            surfaces: Some(&surfaces),
            left,
            now,
            dt,
        });
        crate::frame_timing::projection(projecting.elapsed());
        if let Some(change) = projected.wall.clone() {
            feedback.emit_on(feedback::Event::Hover, projected.wall_hand.into(), now);
            conversation.set_wall(change);
        }
        if let Some(change) = projected.lock.clone() {
            feedback.emit_on(feedback::Event::Choose, projected.wall_hand.into(), now);
            conversation.lock_wall(change);
        }
        if projected.wall_ticked {
            feedback.emit_on(feedback::Event::Hover, projected.wall_hand.into(), now);
        }
        if projected.turned {
            feedback.emit(feedback::Event::Hover, now);
        }
        // Taking hold with the second hand is felt on it, as taking hold is, and so is reaching the least or the most both
        // hands may size what they hold.
        for (event, side) in [
            (feedback::Event::Choose, projected.joined),
            (feedback::Event::Hover, projected.bounded),
        ] {
            if let Some(side) = side {
                feedback.emit_on(event, side.into(), now);
            }
        }
        for side in Side::BOTH {
            if self.projector.held_in(side).is_some() && !held_before[side] {
                feedback.emit_on(feedback::Event::Choose, side.into(), now);
            }
        }
        for (side, placed) in [
            (Side::Right, projected.placed.clone()),
            (Side::Left, projected.left.placed.clone()),
        ] {
            if let Some((panel, target)) = placed {
                feedback.emit_on(feedback::Event::Hover, side.into(), now);
                conversation.place_panel(panel, target);
            }
        }
        // Each hand's view of what is projected: what its ray points at, the item it meets, and where it ends.
        // Whether the right ray points at `panel` on what is projected, rather than carrying it, where a chart's readout
        // is drawn.
        let pointed_here = |panel: &Option<String>| {
            panel.is_some()
                && panel.as_deref() == projected.pointed.as_ref().map(|(id, ..)| id.as_str())
        };
        let views = Hands {
            right: (
                projected.pointed.clone(),
                projected.item,
                projected.hit,
                projected.part.clone(),
                projected.chip.clone(),
            ),
            left: (
                projected.left.pointed.clone(),
                projected.left.item,
                projected.left.hit,
                projected.left.part.clone(),
                projected.left.chip.clone(),
            ),
        };
        // The space's cards stand wherever it shows them; each ray and grip acts on them only while nothing else takes them, such as a panel that hand carries. See `design/github.md`.
        let carded = Hands::from_fn(|side| self.board.held_in(side).is_some());
        let board = self.board.update(crate::board::Input {
            cards: conversation.cards().filter(|_| !in_arrival),
            hands: Hands::from_fn(|side| {
                let carrying = self.projector.carrying_in(side);
                crate::board::Reach {
                    ray: casts[side],
                    grip: tomori_grips[side].filter(|_| !carrying),
                    active: points[side] && !carrying,
                }
            }),
            viewer: senses.viewer,
            now,
        });
        for side in Side::BOTH {
            match (carded[side], self.board.held_in(side).is_some()) {
                (false, true) => feedback.emit_on(feedback::Event::Choose, side.into(), now),
                (true, false) => feedback.emit_on(feedback::Event::Hover, side.into(), now),
                _ => {}
            }
        }
        for (card, placement) in board.placed.iter().cloned() {
            conversation.place_card(card, placement);
        }
        // The environment's client side, in a space whose scenery has grown in, made for the space and environment it
        // is in. It points at what the environment shows only where nothing else is nearer, and not while a card is
        // carried.
        let shown = conversation.client_side().filter(|(environment, _)| {
            scenery.reveal.is_none() && scenery.environment == *environment
        });
        let space = conversation
            .joined_space()
            .map(|(space, _)| space.id.clone());
        // The environment's ambience, too, once its scenery has grown in, carried into the space as it is drawn; and
        // none outside a space, on the grid or in an environment without one.
        let ambience = conversation
            .ambience()
            .filter(|(environment, _)| {
                scenery.reveal.is_none() && scenery.environment == *environment
            })
            .zip(space.clone());
        let playing = ambience
            .as_ref()
            .map(|((environment, _), space)| (space.clone(), *environment, arrangement.carriage));
        if playing != self.ambience {
            self.ambience = playing;
            let carriage = arrangement.carriage;
            feedback.set_ambience(ambience.map(|((_, ambience), _)| {
                std::sync::Arc::new(
                    ambience
                        .carried(|position| carriage.place(Vec3::from_array(position)).to_array()),
                )
            }));
        }
        match (shown, space) {
            (Some((environment, factory)), Some(space)) => {
                let current = self.client.as_ref().is_some_and(|client| {
                    client.space == space && client.environment == environment
                });
                if !current {
                    let side = factory(&space);
                    self.client = Some(Client {
                        space,
                        environment,
                        side,
                        focus: None,
                        select: None,
                        back: false,
                        scrollable: Hands::default(),
                        scroller: Default::default(),
                        reachable: Hands::default(),
                        grabbable: Hands::default(),
                    });
                    self.selector.abandon();
                    self.abandon_scroll();
                }
            }
            _ => {
                if self.client.take().is_some() {
                    self.selector.abandon();
                    self.abandon_scroll();
                }
            }
        }
        // What each ray is on: the nearest of what it meets, Tomori's own interface first. See `design/interaction.md`.
        let mut resolved: Hands<Resolved> = Hands::default();
        for side in Side::BOTH {
            let ray = casts[side];
            let in_space = points[side];
            let (panel_pointed, panel_item, projected_hit, panel_part, panel_chip) =
                views[side].clone();
            let worker = self
                .delegation_scene
                .point_with(side, ray.filter(|_| in_space))
                .map(str::to_owned);
            let hit_point = self.delegation_scene.hit_point_by(side);
            // The nearer of the card and the worker the ray meets is pointed at; a card carried is pointed at all along.
            let reach = |point: Vec3| ray.map_or(f32::INFINITY, |ray| point.distance(ray.origin));
            let card_pointed = board.pointed[side].clone();
            let held_card = self.board.held_in(side).map(str::to_owned);
            let card = held_card.clone().or_else(|| {
                card_pointed
                    .as_ref()
                    .filter(|(_, distance, _)| {
                        hit_point.is_none_or(|point| *distance < reach(point))
                    })
                    .map(|(card, _, _)| card.clone())
            });
            let worker = worker.filter(|_| card.is_none());
            // Or a timer's dial, nearer than any worker. See `design/timers.md`.
            let timer = self
                .delegation_scene
                .pointed_timer_by(side)
                .filter(|_| card.is_none())
                .map(str::to_owned);
            // Or the "+N more" sign of a cluster, which stands for the workers gathered into it.
            let gathered = self
                .delegation_scene
                .pointed_gathered_by(side)
                .filter(|_| card.is_none())
                .map(str::to_owned);
            let nearest = card
                .as_ref()
                .and_then(|_| card_pointed.as_ref().map(|(_, distance, _)| *distance))
                .or_else(|| worker.as_ref().or(timer.as_ref()).and(hit_point).map(reach))
                .unwrap_or(f32::INFINITY);
            // A ray whose hand points down, which points at nothing else, still meets what the environment has that the
            // grip takes, as a stone at one's feet, and shows while it does, as it does a pad: for the grip alone, so that
            // a hand hanging at one's side selects and scrolls nothing.
            let lowered = rays[side].filter(|_| ray.is_none() && self.hands[side].lowered);
            let picked = self
                .client
                .as_ref()
                .zip(ray.or(lowered).filter(|_| in_space && held_card.is_none()))
                // The client side picks in the environment's own coordinates.
                .and_then(|(client, ray)| {
                    client
                        .side
                        .pick_by(side.into(), &arrangement.carriage.unplace_ray(&ray))
                })
                .filter(|picked| {
                    ray.is_some() || picked.actions.contains(crate::client_side::Actions::GRAB)
                })
                .map(|picked| {
                    if ray.is_some() {
                        picked
                    } else {
                        crate::client_side::Picked {
                            selectable: false,
                            actions: crate::client_side::Actions::GRAB,
                            ..picked
                        }
                    }
                });
            let object = picked.clone().filter(|picked| picked.distance < nearest);
            let (card, worker, timer) = if object.is_some() {
                (None, None, None)
            } else {
                (card, worker, timer)
            };
            // A panel is pointed at where nothing else the ray meets is nearer; a panel carried is pointed at all along.
            let nearest = object.as_ref().map_or(nearest, |picked| picked.distance);
            let panel = self.projector.held_in(side).map(str::to_owned).or_else(|| {
                panel_pointed
                    .as_ref()
                    .filter(|(_, distance, _)| *distance < nearest && held_card.is_none())
                    .map(|(panel, _, _)| panel.clone())
            });
            let (card, worker, timer, object) = if panel.is_some() {
                (None, None, None, None)
            } else {
                (card, worker, timer, object)
            };
            // A teleport pad is pointed at, while travel may be used and nothing is carried, where the ray meets nothing
            // else nearer that it may point at or take hold of: a card, a worker, an environment's object, or what the
            // projector meets: a panel pointed at or a wall's board and handles.
            let met = [
                card_pointed.as_ref().map(|(_, distance, _)| *distance),
                hit_point.map(reach),
                picked.as_ref().map(|picked| picked.distance),
                projected_hit.map(|hit| hit.distance),
            ]
            .into_iter()
            .flatten()
            .fold(f32::INFINITY, f32::min);
            let pad = pads_met[side].filter(|(_, distance, _)| *distance < met);
            let (card, worker, timer, object, panel) = if pad.is_some() {
                (None, None, None, None, None)
            } else {
                (card, worker, timer, object, panel)
            };
            // What the ray is on, Tomori's own interface first, which the trigger and the sticks act on next frame: a
            // wall's board, bar or handles, where the participant may take hold of walls, or a wall carried, count as
            // Tomori's too.
            let on_wall = pad.is_none()
                && projected_hit.is_some_and(|hit| {
                    hit.held
                        || picked
                            .as_ref()
                            .is_none_or(|picked| hit.distance < picked.distance)
                });
            let menus = menu_shown && !(side == Side::Left && menu_on_palm);
            let aiming = side == aimer && travel.aiming;
            let aim = if menus
                || on_wall
                || (side == Side::Right && wrist_active)
                || aiming
                || self.hands[side].highlighted.is_some()
                || panel.is_some()
                || card.is_some()
                || worker.is_some()
                || timer.is_some()
            {
                precedence::Aim::Tomori
            } else if pad.is_some() {
                precedence::Aim::Pad
            } else if object.is_some() {
                precedence::Aim::Environment
            } else {
                precedence::Aim::Nothing
            };
            // The checklist item the ray meets on the panel pointed at, not one carried, which A ticks or unticks next,
            // or, a list's choosable row, chooses: a target of its own.
            let item = match (&panel, &panel_pointed, self.projector.held_in(side)) {
                (Some(panel), Some((pointed, ..)), None) if pointed == panel => {
                    panel_item.map(|line| (panel.clone(), line))
                }
                _ => None,
            };
            // The ray ends on what it meets, of every kind it may point at, the nearest, or on what is held: a card or
            // a panel carried, a wall carried or a handle dragged. See `design/interaction.md`.
            let end = rays[side].and_then(|ray| {
                let card_end = match &held_card {
                    Some(_) => {
                        board.end[side].map(|end| crate::pointing::End::held(ray.origin, end))
                    }
                    None => card_pointed
                        .as_ref()
                        .map(|(_, distance, point)| crate::pointing::End::new(*distance, *point)),
                };
                let object_end = picked.as_ref().map(|picked| {
                    crate::pointing::End::new(picked.distance, ray.at(picked.distance))
                });
                let worker_end = hit_point
                    .map(|point| crate::pointing::End::new(point.distance(ray.origin), point));
                let pad_end =
                    pad.map(|(_, distance, point)| crate::pointing::End::new(distance, point));
                crate::pointing::end([card_end, projected_hit, object_end, worker_end, pad_end])
            });
            // What words begun now would name of it.
            let target = if panel.is_some() {
                let item = panel_pointed
                    .as_ref()
                    .filter(|(pointed, ..)| panel.as_deref() == Some(pointed.as_str()))
                    .and(panel_item);
                // A row of an environment's list is named by its id too, so that the agent is told of the row the
                // participant saw, even as the list changes.
                let row = panel
                    .as_deref()
                    .zip(item)
                    .and_then(|(panel, line)| conversation.row_at(panel, line))
                    .map(|row| row.id.clone());
                // On a chart's plot, pointed at with the right ray, whose readout the projector draws, where across
                // its time axis, so that the agent is told the time and values there.
                let across = projected
                    .across
                    .filter(|_| side == Side::Right && pointed_here(&panel));
                // On a volume, the node or edge the ray meets of the volume it points at.
                let part = panel_part.filter(|_| {
                    panel_pointed
                        .as_ref()
                        .is_some_and(|(pointed, ..)| panel.as_deref() == Some(pointed.as_str()))
                });
                // On a chip of its context, the chip's parameter, so that the agent is told what it shows.
                let chip = panel_chip.clone().filter(|_| {
                    panel_pointed
                        .as_ref()
                        .is_some_and(|(pointed, ..)| panel.as_deref() == Some(pointed.as_str()))
                });
                panel
                    .clone()
                    .map(|panel| match (item, row, across, part, chip) {
                        (.., Some(chip)) => tomori_protocol::Resource::panel_chip(panel, chip),
                        (_, _, _, Some(crate::projection::Part::Node(node)), _) => {
                            tomori_protocol::Resource::volume_at(panel, Some(&node), None)
                        }
                        (_, _, _, Some(crate::projection::Part::Edge(edge)), _) => {
                            tomori_protocol::Resource::volume_at(panel, None, Some(edge))
                        }
                        (_, _, _, Some(crate::projection::Part::WorldEdge(edge)), _) => {
                            tomori_protocol::Resource::world_edge_at(panel, edge)
                        }
                        (_, _, Some(across), None, _) => {
                            tomori_protocol::Resource::chart_at(panel, across)
                        }
                        (Some(line), Some(row), ..) => {
                            tomori_protocol::Resource::panel_row(panel, line, row)
                        }
                        _ => tomori_protocol::Resource::panel_at(panel, item),
                    })
            } else if let Some(card) = &card {
                Some(tomori_protocol::Resource::card(card))
            } else if let Some(worker) = &worker {
                Some(tomori_protocol::Resource::worker(worker))
            } else if let Some(timer) = &timer {
                Some(tomori_protocol::Resource::timer(timer))
            } else if let Some(gathered) = &gathered {
                Some(tomori_protocol::Resource::gathered(gathered))
            } else {
                object
                    .as_ref()
                    .map(|picked| tomori_protocol::Resource::object(&picked.object))
            };
            resolved[side] = Resolved {
                card,
                worker,
                timer,
                object,
                panel,
                pad,
                aim,
                item,
                end,
                target,
                card_reach: card_pointed
                    .as_ref()
                    .map_or(f32::INFINITY, |(_, distance, _)| *distance),
            };
        }
        for side in Side::BOTH {
            let resolved = &resolved[side];
            let hand = &mut self.hands[side];
            hand.aim = resolved.aim;
            hand.card_reach = resolved.card_reach;
            // What the trigger and A choose next frame.
            hand.pad = resolved.pad.map(|(key, _, _)| key);
            hand.item = resolved.item.clone();
            // The panel pointed at, not one carried, which the right stick and the wheel scroll next.
            hand.panel = resolved
                .panel
                .clone()
                .filter(|_| self.projector.held_in(side).is_none());
            // Words name what was pointed at most recently first.
            if hand.target != resolved.target {
                hand.since = resolved.target.as_ref().map(|_| now);
                hand.target = resolved.target.clone();
            }
        }
        // The choosable row each ray is on now, which ticks as it is entered and its label offers.
        let rows_now = Hands::from_fn(|side| {
            resolved[side]
                .item
                .as_ref()
                .and_then(|item| conversation.choosable_row(item))
                .filter(|_| conversation.ticks())
        });
        // The buttons on the right controller act on what the right ray is on, or else the left's.
        let serving = self.serving(Side::Right);
        conversation.point_pad(resolved[serving].pad.is_some());
        conversation.point_item(resolved[serving].item.clone());
        // What the trigger and B act on next frame, the right ray's alone in the environment for now.
        let right_object = resolved.right.object.clone();
        if let Some(client) = self.client.as_mut() {
            client.focus = right_object
                .as_ref()
                .filter(|picked| picked.selectable)
                .map(|picked| picked.object.clone());
            // What a push of each hand's stick takes next frame.
            client.scrollable = Hands::from_fn(|side| {
                resolved[side]
                    .object
                    .as_ref()
                    .filter(|picked| picked.actions.contains(crate::client_side::Actions::SCROLL))
                    .map(|picked| picked.object.clone())
            });
            // And what a squeeze of each grip takes: what is within reach of that hand, or else what its ray is on that
            // takes the grip.
            client.grabbable = Hands::from_fn(|side| {
                resolved[side]
                    .object
                    .as_ref()
                    .filter(|picked| picked.actions.contains(crate::client_side::Actions::GRAB))
                    .map(|picked| (picked.object.clone(), picked.distance))
            });
            let side = &client.side;
            client.reachable = Hands::from_fn(|hand| {
                senses.controllers[controller_of(hand)]
                    .filter(|pose| world[hand] && pose.valid())
                    .and_then(|pose| {
                        side.within_reach_of(
                            hand.into(),
                            arrangement
                                .carriage
                                .unplace(Vec3::from_array(pose.position)),
                        )
                    })
            });
        }
        // Each ray shown: in full while it meets something, its hand is busy or aims a teleport, and for a second after;
        // otherwise a short stub from the controller; hidden while its hand points down, holds the menu or the
        // transcript on its palm, or is not tracked. The hand menu's ray is drawn in full, as ever. See
        // `design/interaction.md`.
        let ray_shown = Hands::from_fn(|side| {
            let palm = match side {
                Side::Left => menu_on_palm,
                Side::Right => wrist_shown,
            };
            rays[side].is_some()
                && !palm
                && (casts[side].is_some()
                    || resolved[side].pad.is_some()
                    || resolved[side].object.is_some())
        });
        for side in Side::BOTH {
            // Entering a new target, an arrival object, a hand menu row, a card, a worker, a panel or a list's choosable
            // row on it, ticks very lightly on the hand whose ray entered it, but not while the trigger pulls the swarm;
            // leaving a row for the rest of its panel does not. While a teleport is aimed, only finding a landing ticks
            // the left, even with a panel carried.
            let resolved = &resolved[side];
            let highlighted = self.hands[side].highlighted.clone();
            let target = if side == aimer && travel.aiming {
                travel.landing.map(|_| feedback::key("landing"))
            } else {
                highlighted
                    .as_ref()
                    .map(feedback::key)
                    .or_else(|| resolved.card.as_ref().map(feedback::key))
                    .or_else(|| resolved.worker.as_ref().map(feedback::key))
                    .or_else(|| {
                        resolved
                            .timer
                            .as_ref()
                            .map(|timer| feedback::key(&("timer", timer)))
                    })
                    .or_else(|| {
                        resolved
                            .object
                            .as_ref()
                            .map(|picked| feedback::key(&picked.object))
                    })
                    .or_else(|| resolved.panel.as_ref().map(feedback::key))
                    .or_else(|| resolved.pad.map(|(key, _, _)| feedback::key(&("pad", key))))
            };
            let row = rows_now[side]
                .as_ref()
                .filter(|_| {
                    target.is_some() && target == resolved.panel.as_ref().map(feedback::key)
                })
                .map(feedback::key);
            let pulling = attracts[side];
            let hand = &mut self.hands[side];
            let entered_row = hand.row_hover.update(row, pulling);
            if hand.hover.update(target, pulling) | entered_row {
                feedback.emit_on(feedback::Event::Hover, side.into(), now);
            }
            // The ray has something to say while it meets something, its hand is busy, or it aims a teleport.
            let says = resolved.end.is_some()
                || pointings[side].hit.is_some()
                || busy[side]
                || (side == aimer && travel.aiming);
            if says && ray_shown[side] {
                hand.lit = Some(now);
            }
            hand.full = ray_shown[side]
                && hand
                    .lit
                    .is_some_and(|lit| now.saturating_duration_since(lit) < LIT);
        }
        // The labels on the rays name the panel each points at, not one carried, and say how to tick the item it meets; on
        // a chart's plot, the right ray's readout says what the chart reads there instead, over its face.
        let readout = projected
            .readout
            .clone()
            .filter(|_| pointed_here(&resolved.right.panel));
        let mut projection = projected.sheets;
        for side in Side::BOTH {
            let (panel_pointed, ..) = &views[side];
            let resolved = &resolved[side];
            if side == Side::Right
                && let Some(readout) = readout.clone()
            {
                projection.push(readout);
                projection.extend(projected.linked.iter().cloned());
            } else if let (Some(panel), Some((_, _, point)), None) =
                (&resolved.panel, panel_pointed, self.projector.held_in(side))
                && let Some(shown) = conversation
                    .projection()
                    .and_then(|projection| projection.panel(panel))
            {
                // A choosable row says how to choose it, or, just chosen, that it was; a checklist item how to tick it.
                let chosen = rows_now[side].as_ref().map(|(panel, row)| {
                    if conversation.just_chose(panel, row) {
                        crate::projection::ItemAction::Chosen
                    } else {
                        crate::projection::ItemAction::Choose
                    }
                });
                let tick = chosen
                    .or_else(|| {
                        resolved
                            .item
                            .as_ref()
                            .filter(|_| conversation.ticks())
                            .and_then(|(panel, line)| conversation.item_checked(panel, *line))
                            .map(|checked| {
                                if checked {
                                    crate::projection::ItemAction::Untick
                                } else {
                                    crate::projection::ItemAction::Tick
                                }
                            })
                    })
                    .map(|action| (self.controls, action));
                projection.push(self.hands[side].ray_label.sheet(
                    shown,
                    (*point, senses.viewer.unwrap_or(arrival::HOME)),
                    self.projector.scrollable(panel).then_some(self.controls),
                    tick,
                ));
            }
            // The label beside a teleport pad or target pointed at says where it goes.
            projection.extend(resolved.pad.and_then(|(way, _, point)| {
                crate::pads::label_for(
                    &locomotion,
                    way,
                    point,
                    senses.viewer.unwrap_or(arrival::HOME),
                    self.controls,
                )
            }));
        }
        // The hints beside panels carried, over everything else.
        projection.extend(projected.hint.clone());
        projection.extend(projected.left.hint.clone());
        // The images the panels drawn show, of those held, for the renderer to decode and draw over them.
        let images = projection
            .iter()
            .filter_map(|sheet| {
                sheet
                    .canvas
                    .as_deref()
                    .and_then(|canvas| canvas.image.as_ref())
            })
            .filter_map(|image| {
                conversation
                    .images()
                    .png(&image.hash)
                    .map(|png| (image.hash.clone(), png.clone()))
            })
            .collect();
        // What each shown ray points at, which words begun now name, the one moved onto most recently first.
        for side in Side::BOTH {
            if let Some(picked) = &resolved[side].object {
                conversation.label_object(&picked.object, &picked.label);
            }
        }
        let attended = |side: Side| {
            self.hands[side]
                .target
                .clone()
                .filter(|_| side == Side::Right || ray_shown[side])
        };
        let left_first = match (self.hands.left.since, self.hands.right.since) {
            (Some(left), Some(right)) => left > right,
            _ => false,
        } && ray_shown.left;
        conversation.point(attended(Side::Right));
        conversation.point_left(attended(Side::Left), left_first);
        // What the environment's server side sent reaches its client side, which says what to ask, what it sounds and
        // what its scenery draws.
        let drawn = match self.client.as_mut() {
            Some(client) => {
                let environment = client.environment;
                let updates =
                    conversation.environment_updates(environment.id(), environment.version());
                let head = senses.head_pose.map(|pose| {
                    arrangement
                        .carriage
                        .unplace(Vec3::from_array(pose.position))
                });
                let (left, right) = self.environment_sticks;
                // The push an object of its takes now, of the right stick or else of the wheel, which moves on only
                // once it is told, after the push before it has been told its end.
                let wheeled = match self.wheel_push.as_mut() {
                    Some(wheel) if client.scroller.tells(wheel.number) => Some(wheel.advance(dt)),
                    _ => None,
                };
                if wheeled == Some(None) {
                    self.wheel_push = None;
                }
                let stuck = match (&self.right_stick, &self.left_stick) {
                    (Push::Object(object, number), _) | (_, Push::Object(object, number)) => {
                        Some((object, number))
                    }
                    _ => None,
                };
                let push = match (stuck, &self.wheel_push) {
                    (Some((object, number)), _) => {
                        Some((object.as_str(), *number, self.scroll_axis))
                    }
                    (_, Some(wheel)) => Some((
                        wheel.object.as_str(),
                        wheel.number,
                        wheeled.flatten().unwrap_or_default(),
                    )),
                    _ => None,
                };
                let scroll = client.scroller.update(push);
                // The hand that holds what the grip took: the right controller's grip pose, or, without one, as on the
                // desktop, a point along the ray as far as the ray took it, facing along the ray; in the environment's
                // own coordinates.
                let carriage = arrangement.carriage;
                let unturn = carriage.orientation.inverse();
                let controller = |side: Side| {
                    senses.controllers[controller_of(side)]
                        .filter(|pose| pose.valid())
                        .map(|pose| {
                            crate::client_side::Hand::new(
                                carriage.unplace(Vec3::from_array(pose.position)),
                                unturn * glam::Quat::from_array(pose.orientation).normalize(),
                            )
                        })
                };
                let along = |ray: Option<Ray>| {
                    move |distance: f32| {
                        ray.map(|ray| {
                            let ray = carriage.unplace_ray(&ray);
                            crate::client_side::Hand::new(
                                ray.at(distance.clamp(0.3, 1.2)),
                                glam::Quat::from_rotation_arc(Vec3::NEG_Z, ray.direction),
                            )
                        })
                    }
                };
                let grab = told(
                    &mut self.hands.right.gripping,
                    controller(Side::Right),
                    along(senses.ray),
                    self.hands.right.grip_closed,
                    Side::Right,
                );
                let left_grab = told(
                    &mut self.hands.left.gripping,
                    controller(Side::Left),
                    along(senses.left_ray),
                    self.hands.left.grip_closed,
                    Side::Left,
                );
                // The others' hands, each drawn between the presence updates that bring it, about a frame behind.
                let (hands, left_hands) =
                    crate::relayed::others(&mut self.relayed, conversation, carriage, now);
                let output = client.side.update(
                    crate::client_side::Input::new(updates, now)
                        .pointing(
                            right_object.as_ref().map(|picked| picked.object.as_str()),
                            head,
                        )
                        .with_select(client.select.take())
                        .with_back(std::mem::take(&mut client.back))
                        .with_sticks(left, right)
                        .with_scroll(scroll)
                        .with_grab(grab)
                        .with_left_grab(left_grab)
                        .with_hands(hands)
                        .with_left_hands(left_hands)
                        .as_participant(
                            conversation
                                .source()
                                .filter(|_| conversation.role().participates()),
                        ),
                );
                for intent in output.intents {
                    conversation.environment_intent(
                        environment.id(),
                        environment.version(),
                        intent,
                    );
                }
                for (sound, position) in output.sounds {
                    let position = arrangement.carriage.place(Vec3::from_array(position));
                    feedback.ambient(sound, position.to_array(), now);
                }
                // An object passed from one hand to the other is felt on the hand that took it.
                for side in output.passed {
                    feedback.emit_on(feedback::Event::Passed(side), side, now);
                }
                output.drawn
            }
            None => None,
        };
        // The time the timers' dials count down from, by this client's clock.
        let unix = self.unix(now);
        let (root, workspace) = self.workers.update(conversation.joined_space());
        // Workers fly out from the agent and back to it, wherever it stands.
        self.delegation_scene
            .set_coordinator_samples(self.coordinator.departures());
        // While the agent is away from home, its satellites orbit it, where it stood as of the last frame.
        let bounds = self.coordinator.bounds();
        self.delegation_scene
            .set_agent(arrangement.away().then_some((bounds.centre, bounds.radius)));
        self.delegation_scene
            .seat(&arrangement.clusters, &arrangement.layouts);
        self.delegation_scene.arrange_as(
            arrangement.workers,
            arrangement.audience,
            arrangement.extent,
            (arrangement.formation, arrangement.sweep),
            arrangement.placement,
        );
        // The space's timers, as dials at the agent's home, clear of its reach.
        let agent = match self.coordinator.shown() {
            Some(_) => Vec3::splat(bounds.radius),
            None => crate::dials::SWARM_REACH,
        };
        self.delegation_scene.set_walls(
            conversation
                .projection()
                .map_or(&[][..], |projection| &projection.walls),
        );
        self.delegation_scene.set_timers(
            conversation
                .joined_space()
                .map_or(&[][..], |(space, _)| &space.timers),
            arrangement.home,
            agent,
            unix,
        );
        self.delegation_scene.update(root, workspace, dt);
        // Each ray is drawn where its platform draws rays: at arrival's objects or the hand menu, and in a space at what it meets, where it ends on what it meets, or else as a stub. See `design/interaction.md`.
        let beams = Hands::from_fn(|side| {
            let ray = rays[side].filter(|_| senses.beam && ray_shown[side]);
            let lit = self.hands[side]
                .lit
                .is_some_and(|lit| now.saturating_duration_since(lit) < LIT);
            let stub = |ray: Ray| crate::pointing::Beam {
                origin: ray.origin,
                end: ray.at(STUB),
                hit: false,
            };
            let menus = menu_shown && !(side == Side::Left && menu_on_palm);
            if menus {
                // The hand menu's ray, in full, as ever.
                ray.zip(pointings[side].endpoint(crate::pointing::RAY_LENGTH))
                    .map(|(ray, end)| crate::pointing::Beam {
                        origin: ray.origin,
                        end,
                        hit: pointings[side].hit.is_some(),
                    })
            } else if in_arrival {
                ray.map(
                    |ray| match pointings[side].endpoint(crate::pointing::RAY_LENGTH) {
                        Some(end) if lit => crate::pointing::Beam {
                            origin: ray.origin,
                            end,
                            hit: pointings[side].hit.is_some(),
                        },
                        _ => stub(ray),
                    },
                )
            } else if side == aimer && travel.aiming {
                // Aiming a teleport, it ends at the landing, highlighted, or is plain and short where there is nowhere to land.
                ray.map(|ray| crate::pointing::Beam {
                    origin: ray.origin,
                    end: travel
                        .landing
                        .unwrap_or_else(|| ray.at(crate::pointing::SPACE_RAY_LENGTH)),
                    hit: travel.landing.is_some(),
                })
            } else {
                let hit = resolved[side].end.map(|end| end.point);
                ray.filter(|_| world[side]).map(|ray| match hit {
                    Some(end) => crate::pointing::Beam {
                        origin: ray.origin,
                        end,
                        hit: true,
                    },
                    None if lit => crate::pointing::Beam {
                        origin: ray.origin,
                        end: ray.at(crate::pointing::SPACE_RAY_LENGTH),
                        hit: false,
                    },
                    None => stub(ray),
                })
            }
        });
        // Arrival's objects are still drawn as they leave on entering a space.
        let arrival = (in_arrival
            || menu_shown
            || beams.right.is_some()
            || beams.left.is_some()
            || !scene.objects.is_empty()
            || scene.heading.is_some())
        .then(|| Arrival {
            hovered: pointings
                .right
                .hit
                .as_ref()
                .or(pointings.left.hit.as_ref())
                .map(|hit| hit.key.clone()),
            ray: beams.right,
            left_ray: beams.left,
            time: self.arrival_time,
            scene,
        });
        // Every source is heard from where it is, relative to the head; see `design/spatial-audio.md`.
        let agent = self.coordinator.voice();
        // The others in a space see this participant's head, controllers and hands, in the space's scene coordinates, and the server alone the panels they carry and which rays they show. Visitors send no presence; see `design/security.md`.
        let presence = senses
            .head_pose
            .filter(|_| conversation.role().participates())
            .map(|head| Presence {
                head,
                left: senses.controllers[crate::controller::LEFT],
                right: senses.controllers[crate::controller::RIGHT],
                hands: senses.hands,
                carried: Side::BOTH
                    .into_iter()
                    .filter_map(|side| self.projector.held_in(side).map(str::to_owned))
                    .collect(),
                rays: [ray_shown.left, ray_shown.right],
            });
        // Arrival hides the panel; entering a space places it in front of the participant again.
        if in_arrival {
            self.panel.reset();
        }
        let panel_pose = self.panel.update(senses.space_head, dt);
        // Arrival has no pill, and it hides while a hand's panel shows, so that it does not cover it.
        let pill = controller_ui::pill(
            conversation,
            now,
            in_arrival || menu_shown || wrist_shown,
            panel_pose,
        );
        let pointed_pad = self.hands.right.pad.or(self.hands.left.pad);
        Frame {
            scenery,
            swarm: self.coordinator.swarm(),
            marker,
            attracting,
            card: self.presentation_navigation.card,
            content: self.presentation_navigation.content,
            cubes: self.delegation_scene.cubes(),
            stations: self.delegation_scene.stations(),
            dials: self.delegation_scene.dials(),
            sign_facing: self.sign_facing.update(senses.viewer, dt),
            workspace,
            arrival,
            panels: Panels {
                pill,
                indicator,
                wrist,
            },
            listener: senses.head_pose,
            agent,
            presence,
            cards: board.sheets,
            projection,
            images,
            volumes: projected.volumes.clone(),
            landing: travel.landing.or_else(|| {
                crate::pads::ring(&locomotion, pointed_pad).filter(|_| in_space_shown && grown)
            }),
            blink: travel.blink,
            pads: if in_space_shown && grown {
                crate::pads::shown(&locomotion, pointed_pad)
            } else {
                Vec::new()
            },
            environment: drawn,
            carriage,
            avatar: self.coordinator.shown(),
            home_ring,
        }
    }
}

/// What one hand's ray is on this frame, of everything it meets, Tomori's own interface first.
#[derive(Clone, Debug, Default)]
struct Resolved {
    card: Option<String>,
    worker: Option<String>,
    timer: Option<String>,
    object: Option<crate::client_side::Picked>,
    panel: Option<String>,
    pad: Option<(crate::pads::Way, f32, Vec3)>,
    aim: precedence::Aim,
    item: Option<(String, u32)>,
    end: Option<crate::pointing::End>,
    target: Option<tomori_protocol::Resource>,
    card_reach: f32,
}

/// Conversations and feedback in the states the frame step's tests need.
#[cfg(any(test, feature = "testing"))]
pub mod fixtures {
    use crate::conversation::Conversation;
    use tomori_client::{
        ConnectionState, Event,
        feedback::{Feedback, Settings},
        server_choice::{Choice, Source},
    };
    use tomori_protocol::{
        Approval, ApprovalOption, ConnectionRole, EnvironmentInfo, KnockRequest, Participant,
        ServerMessage, SpaceState,
    };

    pub const SPACE: &str = "space-1";

    /// Connected to a server as `role`, in the lobby.
    pub fn lobby(role: ConnectionRole) -> Conversation {
        let mut conversation = crate::test_support::conversation("Ada");
        conversation.connect(Choice::new("Studio", "10.0.0.1:8765", Source::Discovered));
        conversation.apply(Event::State(ConnectionState::Connected {
            source: 1,
            server: "studio".into(),
            role,
            certificate: None,
            backends: vec![],
            roots: vec![],
            environments: vec![EnvironmentInfo {
                id: "sky".into(),
                label: "Sky".into(),
                version: 1,
            }],
            utterance_limit: None,
            name: None,
        }));
        conversation.take_actions();
        conversation
    }

    /// The space's state, changed by `change`.
    pub fn space(change: impl FnOnce(&mut SpaceState)) -> Event {
        let mut state = SpaceState {
            id: SPACE.into(),
            title: "Tomori".into(),
            backend: "opencode".into(),
            directory: "/work/tomori".into(),
            environment: "sky".into(),
            participants: vec![Participant {
                name: "Ada".into(),
                source: 1,
                engagement: None,
            }],
            ..Default::default()
        };
        change(&mut state);
        Event::Message(ServerMessage::Space {
            state: Box::new(state),
        })
    }

    /// Joined to [`SPACE`] as its owner.
    pub fn joined() -> Conversation {
        let mut conversation = lobby(ConnectionRole::Owner);
        conversation.apply(space(|_| {}));
        conversation.take_actions();
        conversation
    }

    /// Show `cards` in [`SPACE`], which shows cards, as the server sends them.
    pub fn show_cards(conversation: &mut Conversation, cards: Vec<tomori_protocol::Card>) {
        conversation.apply(space(|state| state.cards = true));
        conversation.apply(Event::Message(ServerMessage::Cards {
            cards: Box::new(tomori_protocol::Cards {
                space: SPACE.into(),
                source: "o/r".into(),
                cards,
                status: None,
            }),
        }));
        conversation.take_actions();
    }

    /// The Tomori wall in [`SPACE`], where the server puts it in an environment that declares no stage.
    pub fn tomori_wall() -> tomori_protocol::Wall {
        tomori_protocol::Wall::new(
            tomori_protocol::TOMORI_WALL,
            tomori_protocol::WallKind::Tomori,
            tomori_protocol::Pose::upright_facing([0.0, 0.52, -2.3], [0.0, 0.52, 0.0]),
        )
        .with_shape(tomori_protocol::WallShape::Arc { radius: 2.3 })
    }

    /// Show `panels` on [`tomori_wall`] and `walls` in [`SPACE`], which projects, as the server sends them.
    pub fn show_projection(
        conversation: &mut Conversation,
        walls: Vec<tomori_protocol::Wall>,
        panels: Vec<tomori_protocol::Panel>,
    ) {
        conversation.apply(space(|state| state.projection = true));
        let mut shown = vec![tomori_wall()];
        shown.extend(walls);
        conversation.apply(Event::Message(ServerMessage::Projection {
            projection: Box::new(
                tomori_protocol::Projection::new(SPACE, "sky")
                    .with_walls(shown)
                    .with_panels(panels),
            ),
        }));
        conversation.take_actions();
    }

    /// Show `panels` on [`tomori_wall`] and `walls` in [`SPACE`], whose server projects and takes wall changes, as the
    /// server sends them.
    pub fn show_editable_walls(
        conversation: &mut Conversation,
        walls: Vec<tomori_protocol::Wall>,
        panels: Vec<tomori_protocol::Panel>,
    ) {
        show_projection(conversation, walls, panels);
        conversation.apply(space(|state| {
            state.projection = true;
            state.walls = true;
            state.groups = true;
        }));
        conversation.take_actions();
    }

    /// Show an agenda, its first item ticked and its second not, at revision 4, floating at `at` facing `viewer`, in
    /// [`SPACE`], whose server projects and takes ticking, as the server sends it.
    pub fn show_checklist(conversation: &mut Conversation, at: glam::Vec3, viewer: glam::Vec3) {
        conversation.apply(space(|state| {
            state.projection = true;
            state.checks = true;
        }));
        conversation.apply(Event::Message(ServerMessage::Projection {
            projection: Box::new(
                tomori_protocol::Projection::new(SPACE, "sky")
                    .with_walls(vec![tomori_wall()])
                    .with_panels(vec![
                        tomori_protocol::Panel::new(
                            "panel-1",
                            "Agenda",
                            "To cover:\n\n- [x] Panel ids\n- [ ] Pairing window",
                            tomori_protocol::Place::Free {
                                pose: tomori_protocol::Pose::upright_facing(
                                    at.to_array(),
                                    viewer.to_array(),
                                ),
                            },
                        )
                        .with_version(4),
                    ]),
            ),
        }));
        conversation.take_actions();
    }

    /// Showing a queue in [`SPACE`], an environment's list panel floating at `at`, facing `viewer`, whose rows `pod112`
    /// and `pod9`, on lines 1 and 2, may be chosen and `pod7`, on line 0, may not, at version 6, by a server that takes
    /// choices and ticks.
    pub fn show_queue(conversation: &mut Conversation, at: glam::Vec3, viewer: glam::Vec3) {
        conversation.apply(space(|state| {
            state.projection = true;
            state.checks = true;
            state.choices = true;
        }));
        conversation.apply(Event::Message(ServerMessage::Projection {
            projection: Box::new(
                tomori_protocol::Projection::new(SPACE, "sky")
                    .with_walls(vec![tomori_wall()])
                    .with_panels(vec![
                        tomori_protocol::Panel::new(
                            "panel-2",
                            "Queue",
                            "- pod7 waiting\n- pod112 waiting\n- pod9 waiting",
                            tomori_protocol::Place::Free {
                                pose: tomori_protocol::Pose::upright_facing(
                                    at.to_array(),
                                    viewer.to_array(),
                                ),
                            },
                        )
                        .with_rows(vec![
                            tomori_protocol::ListRow::new("pod7", false),
                            tomori_protocol::ListRow::new("pod112", true),
                            tomori_protocol::ListRow::new("pod9", true),
                        ])
                        .with_version(6),
                    ]),
            ),
        }));
        conversation.take_actions();
    }

    /// Showing `walls` and `panels` in [`SPACE`], whose server takes wall changes and locks walls.
    pub fn show_lockable_walls(
        conversation: &mut Conversation,
        walls: Vec<tomori_protocol::Wall>,
        panels: Vec<tomori_protocol::Panel>,
    ) {
        show_projection(conversation, walls, panels);
        conversation.apply(space(|state| {
            state.projection = true;
            state.walls = true;
            state.wall_locks = true;
        }));
        conversation.take_actions();
    }

    /// The last row of the label on the ray in `projection`, if there is one: the one with the panel's name, the agenda
    /// of [`show_checklist`] or the queue of [`show_queue`].
    #[cfg(test)]
    pub fn label_hint(projection: &[crate::controller_ui::Sheet]) -> Option<String> {
        projection
            .iter()
            .filter_map(|sheet| sheet.canvas.as_deref())
            .find(|canvas| {
                let plain = canvas.plain();
                (plain.starts_with("Agenda") || plain.starts_with("Queue"))
                    && canvas.rows.len() <= 4
            })
            .and_then(|canvas| {
                canvas
                    .plain()
                    .lines()
                    .last()
                    .map(|line| line.trim_end().to_owned())
            })
    }

    /// Joined to [`SPACE`] with an approval waiting.
    pub fn approval_waiting() -> Conversation {
        let mut conversation = lobby(ConnectionRole::Owner);
        conversation.apply(space(|state| {
            state.approval = Some(Approval {
                id: "request-1".into(),
                title: "Bash".into(),
                text: "cargo test".into(),
                options: vec![ApprovalOption {
                    id: "once".into(),
                    label: "Allow once".into(),
                }],
            });
        }));
        conversation.take_actions();
        conversation
    }

    /// Joined to [`SPACE`], where the participant asked something and the agent replied.
    pub fn conversed() -> Conversation {
        let mut conversation = lobby(ConnectionRole::Owner);
        conversation.apply(space(|state| {
            state.messages = [
                (tomori_protocol::Role::User, "Fix the parser."),
                (tomori_protocol::Role::Assistant, "The parser is fixed."),
            ]
            .into_iter()
            .enumerate()
            .map(|(index, (role, text))| tomori_protocol::Message {
                id: format!("message-{index}"),
                role,
                text: text.into(),
                completed: true,
            })
            .collect();
        }));
        conversation.take_actions();
        conversation
    }

    /// A knock from `name` on the space.
    pub fn knock(conversation: &mut Conversation, id: &str, name: &str) {
        conversation.apply(Event::Message(ServerMessage::Knocks {
            knocks: vec![KnockRequest {
                id: id.into(),
                name: name.into(),
            }],
        }));
    }

    /// Feedback that vibrates, so that what would be felt can be seen.
    pub fn felt() -> Feedback {
        let mut feedback = Feedback::new(Settings::default());
        feedback.set_haptics(true);
        feedback
    }
}

#[cfg(test)]
mod tests;
