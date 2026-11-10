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
    /// The left stick was clicked: mute spoken replies.
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
    pub ray: Option<crate::pointing::Beam>,
    pub time: f32,
}

/// This participant's head, controllers and hands, for the others in the space, and the panel they carry, if any, for
/// the server alone.
#[derive(Clone, Debug)]
pub struct Presence {
    pub head: Pose,
    pub left: Option<Pose>,
    pub right: Option<Pose>,
    pub hands: [Option<Hand>; 2],
    /// The panel carried, by its identity, so that the space's agent does not move it out of the hand.
    pub carrying: Option<String>,
}

impl Presence {
    /// Report it to the server through `connection`, so the others in the space can render it.
    pub fn send(&self, connection: &mut tomori_client::connection::Connection) {
        connection.send_presence(
            self.head.into(),
            self.left.map(Into::into),
            self.right.map(Into::into),
            self.hands,
            self.carrying.as_deref(),
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
}

/// Whose a push of the right stick is: each push belongs to what took it as it left the middle, until it returns there,
/// as [`precedence::Right`] orders.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
enum RightStick {
    /// At the middle.
    #[default]
    Free,
    /// A panel was carried as it left the middle: the push moves and turns it.
    Carried,
    /// Nothing was carried, and the ray pointed at this panel, which has more than it shows: the push scrolls it, for
    /// this participant alone.
    Scrolled(String),
    /// The transcript showed, or the ray was on something else of Tomori's, or on nothing with no environment's client
    /// side to take it: the push moves the transcript's focus or pages cards, as ever.
    Other,
    /// Nothing of Tomori's took it, in a space with an environment's client side: the push is the client side's. See
    /// `design/environments.md`.
    Environment,
    /// Nothing of Tomori's took it, and the ray pointed at this object of the client side's, which takes the right
    /// stick: the push is the object's, told as the client side's `Input::scroll` with the push's number, which no other
    /// push shares. See `design/environments.md`.
    Object(String, u64),
    /// The push was an object's when focus was lost or its client side went: it is nobody's until it returns to the
    /// middle, telling the next client side nothing.
    Abandoned,
}

impl RightStick {
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

/// Whose a push of the left stick is: each push belongs to what took it as it left the middle, until it returns there,
/// as [`precedence::Left`] orders.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum LeftStick {
    /// At the middle.
    #[default]
    Free,
    /// Travel's, where the environment allows it, or nothing's.
    Other,
    /// In a space whose environment has a client side, which allows no travel here, with the ray on nothing of
    /// Tomori's: the client side's. See `design/environments.md`.
    Environment,
}

impl LeftStick {
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
    /// The object of its that the ray pointed at last frame, if it takes the right stick, which a push takes.
    scrollable: Option<String>,
    /// The pushes of the right stick, and turns of the wheel, it has been told of.
    scroller: crate::client_side::Scroller,
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
    /// The arrival object the ray met on the last frame, which A chooses.
    highlighted: Option<arrival::Key>,
    /// Whether the hand menu showed last frame, when A is read, so that A then chooses only in it.
    menu_was_shown: bool,
    hand_menu: HandMenu,
    wrist: Wrist,
    indicator: Indicator,
    pointer: Pointer<arrival::Key>,
    hover: feedback::Hover,
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
    /// How far along the ray the card it met last frame was, which the grip picks up instead of a panel behind it.
    card_reach: f32,
    /// Where the participant has travelled, in an environment that allows it, and the teleport being aimed; see `design/locomotion.md`.
    travel: Travel,
    /// The left stick, which travels in such a space, from the last buttons.
    travel_stick: Vec2,
    /// Whose the right stick's push is, from when it left the middle until it returns.
    right_stick: RightStick,
    /// The right stick, while its push is an object's, from the last buttons.
    scroll_axis: Vec2,
    /// A turn of the desktop's wheel that an object takes, as a push of the right stick. See `design/desktop.md`.
    wheel_push: Option<WheelPush>,
    /// How many pushes have been an object's, which numbers each.
    pushes: u64,
    /// The right stick, while a panel carried takes it, from the last buttons, and whether it was clicked since the last
    /// frame, which turns the panel back to square.
    carry_stick: Vec2,
    straighten: bool,
    /// The panel the right stick scrolls, and how far forward it is pushed, from the last buttons, and the panel the
    /// mouse's wheel scrolled since the last frame, and by how many rows.
    scroll_stick: Option<(String, f32)>,
    wheel: Option<(String, f32)>,
    /// The panel the ray pointed at last frame, not one carried, which a push of the right stick or the wheel scrolls.
    pointed_panel: Option<String>,
    /// The wall carried as B was pressed, and when: removed for everyone, or asked to confirm, if B is let go within
    /// [`crate::conversation::DISMISS_PRESS`] while it is still carried. See `design/projection.md`.
    removing: Option<(String, Instant)>,
    /// What the ray pointed at last frame, Tomori's own interface first, which decides whose the trigger and the sticks
    /// are as they begin. See `design/interaction.md`.
    aim: precedence::Aim,
    /// The panel carried as B was pressed, and when: thrown away for everyone if B is let go within
    /// [`crate::conversation::DISMISS_PRESS`] while it is still carried. See `design/projection.md`.
    throwing: Option<(String, Instant)>,
    /// The controls the hint beside a panel carried names.
    controls: crate::projection::Controls,
    /// The client side of the joined space's environment, for that space and environment. See `design/environments.md`.
    client: Option<Client>,
    /// The left and right sticks, where they are the client side's, from the last buttons.
    environment_sticks: (Vec2, Vec2),
    /// Whose the left stick's push is, from when it left the middle until it returns.
    left_stick: LeftStick,
    /// The pulls of the trigger and the presses of A on the client side's selectable objects, which outlive a client
    /// side, so that a pull begun for one is nobody's once it has gone. See `design/environments.md`.
    selector: crate::client_side::Selector,
    /// What A did, from the last buttons, which the selector is told with the trigger.
    a_press: crate::client_side::Press,
    /// Whose the last press of B was, for tests that check the order.
    #[cfg(test)]
    last_back: Option<crate::conversation::Back>,
    /// The agent's home as the swarm last stood at it: when it changes, as a space opens or changes environment, the
    /// swarm appears at the new one rather than gliding. See `design/environments.md`.
    agent_home: crate::anchors::Anchor,
    /// The pulls of the trigger on teleport pads, with the thresholds that choose an arrival object, and the pad the ray
    /// pointed at last frame, which a pull begun on it, or A, chooses. See `design/locomotion.md`.
    pad_selector: crate::client_side::Selector,
    pointed_pad: Option<crate::pads::Key>,
    /// The pulls of the trigger on choosable rows of an environment's lists, with the same thresholds, a pull begun on
    /// one choosing it once pressed there. See `design/panel-actions.md`.
    row_selector: crate::client_side::Selector,
    /// The rig the platform placed this frame's poses with, as [`Self::rig`] last settled it.
    placed: Rig,
    /// Where the space's environment and its agent stood in the participant's room on the last frame in a space, and,
    /// once it is left, while its environment shrinks back into its object: they stay there until it has gone. See
    /// `design/arrival.md`.
    standing: Option<Standing>,
    departing: bool,
}

/// Where a space's environment and its agent stand in the participant's room: the environment drawn, its carriage and
/// the agent's anchor, each in the room, and the rig they were last placed in the scene with.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Standing {
    environment: crate::environment::Environment,
    carriage: crate::anchors::Anchor,
    agent: crate::anchors::Anchor,
    rig: Rig,
}

impl Default for Step {
    fn default() -> Self {
        Self {
            stick_latched: false,
            back_since: None,
            was_talking: false,
            talk_blocked: false,
            highlighted: None,
            menu_was_shown: false,
            hand_menu: HandMenu::default(),
            wrist: Wrist::default(),
            indicator: Indicator::default(),
            pointer: Pointer::default(),
            hover: feedback::Hover::default(),
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
            card_reach: f32::INFINITY,
            travel: Travel::default(),
            travel_stick: Vec2::ZERO,
            right_stick: RightStick::Free,
            scroll_axis: Vec2::ZERO,
            wheel_push: None,
            pushes: 0,
            carry_stick: Vec2::ZERO,
            straighten: false,
            scroll_stick: None,
            wheel: None,
            pointed_panel: None,
            aim: precedence::Aim::Nothing,
            throwing: None,
            removing: None,
            controls: crate::projection::Controls::default(),
            client: None,
            environment_sticks: (Vec2::ZERO, Vec2::ZERO),
            left_stick: LeftStick::Free,
            selector: crate::client_side::Selector::default(),
            a_press: crate::client_side::Press::default(),
            #[cfg(test)]
            last_back: None,
            agent_home: crate::anchors::Anchor::AGENT,
            pad_selector: crate::client_side::Selector::default(),
            row_selector: crate::client_side::Selector::default(),
            pointed_pad: None,
            placed: Rig::default(),
            standing: None,
            departing: false,
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
        self.pointer.release();
        self.highlighted = None;
        self.presentation_navigation.release();
        self.cancel_travel();
        // A push that was an object's is nobody's until it returns to the middle, so that it does not begin again.
        self.abandon_scroll();
        if self.right_stick != RightStick::Abandoned {
            self.right_stick = RightStick::Free;
        }
        self.carry_stick = Vec2::ZERO;
        self.straighten = false;
        self.scroll_stick = None;
        self.wheel = None;
        self.throwing = None;
        self.removing = None;
        self.environment_sticks = (Vec2::ZERO, Vec2::ZERO);
        self.left_stick = LeftStick::Free;
        self.a_press = crate::client_side::Press::default();
        self.pointed_pad = None;
        if let Some(client) = self.client.as_mut() {
            client.back = false;
        }
    }

    /// A push of the right stick or a turn of the wheel that is an object's ends, as focus is lost or its client side
    /// goes: the push is nobody's until it returns to the middle, and the client side, if it is still there, is told it
    /// ended on its next frame.
    fn abandon_scroll(&mut self) {
        if matches!(self.right_stick, RightStick::Object(..)) {
            self.right_stick = RightStick::Abandoned;
        }
        self.wheel_push = None;
        self.scroll_axis = Vec2::ZERO;
    }

    /// Drop a teleport being aimed, as when the left stick is lost, without travelling.
    pub fn cancel_travel(&mut self) {
        self.travel.cancel();
        self.travel_stick = Vec2::ZERO;
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
            || matches!(self.highlighted, Some(arrival::Key::Menu(_)))
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
        // with the ray on something of its own or on nothing of Tomori's. See `design/interaction.md`.
        let environment = self.client.is_some();
        let aim = self.aim;
        let travels = !matches!(conversation.locomotion(), locomotion::Locomotion::None);
        self.left_stick = self.left_stick.update(stick, || {
            match precedence::Left::first(|rung| match rung {
                precedence::Left::Travel => travels,
                precedence::Left::Tomori => {
                    matches!(aim, precedence::Aim::Tomori | precedence::Aim::Pad)
                }
                precedence::Left::Environment => environment,
                precedence::Left::Nothing => true,
            }) {
                precedence::Left::Environment => LeftStick::Environment,
                _ => LeftStick::Other,
            }
        });
        let (travel_stick, environment_left) = match self.left_stick {
            LeftStick::Environment => (Vec2::ZERO, stick),
            _ => (stick, Vec2::ZERO),
        };
        // In a space whose environment allows it, the left stick travels, which the frame step does with the ray; see `design/locomotion.md`.
        self.travel_stick = if focused { travel_stick } else { Vec2::ZERO };
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
        // While the right wrist's transcript shows, the right stick, A and B act on it, and B backs out of it before the space. See `design/wrist-transcript.md`.
        let wrist_active = self.wrist.state().active();
        // A push of the right stick is the panel or wall carried's, if one is carried as it leaves the middle, until it returns there, and then nothing else's, so that the transcript, which does not show while a panel is carried, or the cards, never take a push that moved the panel, nor the panel one that moved them. Otherwise, while the transcript does not show, a push that leaves the middle pointing at a panel with more than it shows scrolls it, for this participant alone, and nothing else. See `design/projection.md`.
        let carrying = self.projector.carrying();
        let scrolls = self
            .pointed_panel
            .as_deref()
            .filter(|panel| !wrist_active && self.projector.scrollable(panel));
        // An object of the environment's that takes the right stick, pointed at, has a push that nothing of Tomori's
        // takes, with its identity. See `design/environments.md`.
        let object = self
            .client
            .as_ref()
            .and_then(|client| client.scrollable.as_deref());
        let rung = precedence::Right::first(|rung| match rung {
            precedence::Right::Carried => carrying,
            precedence::Right::Transcript => wrist_active,
            precedence::Right::Scroll => scrolls.is_some(),
            precedence::Right::Tomori => {
                matches!(aim, precedence::Aim::Tomori | precedence::Aim::Pad)
            }
            precedence::Right::Object => object.is_some(),
            precedence::Right::Environment => environment,
            precedence::Right::Cards => true,
        });
        let number = self.pushes + 1;
        self.right_stick =
            std::mem::take(&mut self.right_stick).update(buttons.right_stick, || match rung {
                precedence::Right::Carried => RightStick::Carried,
                precedence::Right::Scroll => scrolls.map_or(RightStick::Other, |panel| {
                    RightStick::Scrolled(panel.to_owned())
                }),
                // A push begun without focus is nobody's, so that it never begins an object's.
                precedence::Right::Object => match object {
                    Some(object) if focused => RightStick::Object(object.to_owned(), number),
                    _ => RightStick::Abandoned,
                },
                precedence::Right::Environment => RightStick::Environment,
                precedence::Right::Transcript
                | precedence::Right::Tomori
                | precedence::Right::Cards => RightStick::Other,
            });
        if let RightStick::Object(_, number) = &self.right_stick {
            self.pushes = *number;
            self.wheel_push = None;
        }
        let (carry_stick, right, environment_right) = match self.right_stick {
            RightStick::Carried => (buttons.right_stick, Vec2::ZERO, Vec2::ZERO),
            RightStick::Scrolled(_) | RightStick::Object(..) | RightStick::Abandoned => {
                (Vec2::ZERO, Vec2::ZERO, Vec2::ZERO)
            }
            RightStick::Environment => (Vec2::ZERO, Vec2::ZERO, buttons.right_stick),
            RightStick::Free | RightStick::Other => (Vec2::ZERO, buttons.right_stick, Vec2::ZERO),
        };
        self.scroll_axis = match self.right_stick {
            RightStick::Object(..) => buttons.right_stick,
            _ => Vec2::ZERO,
        };
        self.carry_stick = if focused { carry_stick } else { Vec2::ZERO };
        self.scroll_stick = match &self.right_stick {
            RightStick::Scrolled(panel) if focused => Some((panel.clone(), buttons.right_stick.y)),
            _ => None,
        };
        self.environment_sticks = if focused {
            (environment_left, environment_right)
        } else {
            (Vec2::ZERO, Vec2::ZERO)
        };
        // The mouse's wheel scrolls the panel pointed at, unless one is carried or the transcript shows.
        if let Some(panel) = &self.pointed_panel
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
            && self.right_stick == RightStick::Free
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
        let rung = buttons.a.then(|| {
            precedence::A::first(|rung| match rung {
                precedence::A::MenuRow => matches!(self.highlighted, Some(arrival::Key::Menu(_))),
                precedence::A::Menu => self.menu_was_shown,
                precedence::A::Transcript => wrist_active,
                precedence::A::Conversation => {
                    let nothing = conversation.press_a(self.highlighted.as_ref());
                    chose = !nothing && !conversation.a_refused();
                    !nothing
                }
                precedence::A::Tomori => aim == precedence::Aim::Tomori,
                precedence::A::Environment => focused && focus.is_some(),
                precedence::A::Nothing => true,
            })
        });
        match (rung, &self.highlighted) {
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
        // Left stick click mutes spoken replies locally; right stick click plays the latest reply again, or stops it.
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
        let in_space = conversation.joined_space().is_some();
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
                // A panel carried keeps the transcript away, as pointing does, so that turning the wrist does not let it go.
                pointing: senses.pointing || self.projector.carrying(),
                grip: senses.grip,
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
        // What the ray pointed at last frame, so that the description of the mode pointed at shows beneath the modes, in rows that are there whatever is pointed at, so that the rows do not move under the ray.
        let pointed = match &self.highlighted {
            Some(arrival::Key::Menu(key)) => Some(key.clone()),
            _ => None,
        };
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
        let pointing = self.pointer.update(senses.ray, senses.trigger, &targets);
        self.highlighted = pointing.hit.as_ref().map(|hit| hit.key.clone());
        if let Some(key) = &pointing.chosen {
            feedback.emit(feedback::Event::Choose, now);
            match key {
                arrival::Key::Menu(key) => {
                    if let Some(choice) = self.hand_menu.choose(key) {
                        conversation.choose_menu(choice);
                    }
                }
                key => conversation.choose_object(key),
            }
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
        // Whose a pull of the trigger is, decided as it begins, until it is let go: the first rung of
        // `precedence::Trigger` that claims it, as the ray found things last frame, Tomori's own interface first. A pull
        // that begins on a selectable object of the environment's client side, with nothing of Tomori's taking it, is the
        // client side's, and does not attract the swarm. See `design/interaction.md` and `design/environments.md`.
        let focus = self
            .client
            .as_ref()
            .and_then(|client| client.focus.as_deref());
        let aim = self.aim;
        let carrying_wall = self.projector.carried_wall().is_some();
        // A choosable row of an environment's list the ray met last frame, for a participant: a pull begun on it chooses
        // it, as A does, rather than attracting the swarm. See `design/panel-actions.md`.
        let row = conversation
            .pointed_row()
            .filter(|_| conversation.ticks())
            .map(|(panel, row)| format!("{panel}\n{row}"));
        let rung = precedence::Trigger::first(|rung| match rung {
            precedence::Trigger::Menu => menu_shown || in_arrival,
            precedence::Trigger::Transcript => wrist_active,
            precedence::Trigger::CarriedWall => carrying_wall,
            precedence::Trigger::Row => aim == precedence::Aim::Tomori && row.is_some(),
            precedence::Trigger::Tomori => aim == precedence::Aim::Tomori,
            precedence::Trigger::Pad => aim == precedence::Aim::Pad,
            precedence::Trigger::Environment => focus.is_some(),
            precedence::Trigger::Swarm => true,
        });
        let others = self.pad_selector.holds() || self.row_selector.holds();
        let select = self.selector.update_beside(
            focus.filter(|_| rung == precedence::Trigger::Environment),
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
        let pad_name = self
            .pointed_pad
            .filter(|_| rung == precedence::Trigger::Pad)
            .map(|key| format!("{}/{}", key.pair, key.end));
        let others = self.selector.holds() || self.row_selector.holds();
        let pulled = matches!(
            self.pad_selector.update_beside(
                pad_name.as_deref(),
                senses.trigger,
                crate::client_side::Press::default(),
                others,
            ),
            Some(crate::client_side::Select::Pressed(_))
        );
        let pressed = conversation.take_pad_choice();
        let chosen_pad = self.pointed_pad.filter(|_| pulled || pressed);
        // Pressed as far as choosing takes, on the row it began on, it chooses that row, with the choosing click.
        // One select at a time: a pull that begins while the environment's select or a pad's is held is nobody's.
        let others = self.selector.holds() || self.pad_selector.holds();
        if let Some(crate::client_side::Select::Pressed(_)) = self.row_selector.update_beside(
            row.as_deref().filter(|_| rung == precedence::Trigger::Row),
            senses.trigger,
            crate::client_side::Press::default(),
            others,
        ) && conversation.choose_row()
        {
            feedback.emit(feedback::Event::Choose, now);
        }
        let selecting =
            self.selector.owns() || self.pad_selector.owns() || self.row_selector.owns();
        // A trigger pull that misses the hand menu's rows does nothing while it shows, nor while the transcript shows.
        // While a wall is carried, the trigger keeps it from snapping rather than attracting the swarm. See
        // `design/projection.md`. In a space it draws the swarm nowhere: the agent comes to whoever speaks to it, and the
        // trigger is for pointing and selecting. See `design/interaction.md`.
        let attracting = pointing.attracting
            && !in_space_shown
            && !menu_shown
            && !wrist_active
            && !selecting
            && rung != precedence::Trigger::CarriedWall;
        let attractor = senses.attractor.filter(|_| attracting);
        // The agent appears at its home as a space opens, or changes environment, and otherwise glides to where it goes
        // and back home; the workers stand about its home. See `design/environments.md`.
        let arrangement = conversation.arrangement();
        // Leaving a space, the rig goes home and the space's state is cleared at once, while its environment still
        // shrinks back into the space's object: the environment and the agent stay where the participant saw them, in
        // their room, until it has gone, and only then does the agent appear back at its place, so that nothing jumps
        // as the space is left. See `design/arrival.md`.
        let placed = self.placed;
        let in_a_space = conversation.joined_space().is_some();
        let departing = !in_a_space
            && self
                .standing
                .is_some_and(|standing| standing.environment == scenery.environment);
        let departed = self.departing && !departing;
        self.departing = departing;
        if !in_a_space && !departing {
            self.standing = None;
        }
        let departure = self
            .standing
            .as_mut()
            .filter(|_| departing)
            .map(|standing| {
                // The rig going home moves the scene under the participant: they are carried with it at once.
                let moved = standing.rig != placed;
                standing.rig = placed;
                (
                    placed.place(standing.carriage),
                    placed.place(standing.agent),
                    moved,
                )
            });
        let carriage = departure.map_or(arrangement.carriage, |(carriage, _, _)| carriage);
        let appear = match departure {
            Some((_, _, moved)) => moved,
            None => departed || arrangement.home != self.agent_home,
        };
        self.agent_home = arrangement.home;
        let anchor = departure.map_or(arrangement.agent, |(_, agent, _)| agent);
        // In a space whose environment is drawn, where it and the agent stand in the participant's room, for as long
        // as it shrinks away should the space be left.
        if in_a_space {
            self.standing =
                (scenery.environment != crate::environment::Environment::MINIMAL).then(|| {
                    Standing {
                        environment: scenery.environment,
                        carriage: placed.unplace(arrangement.carriage),
                        agent: placed.unplace(anchor),
                        rig: placed,
                    }
                });
        }
        // The agent stands there as the avatar the space's backend, or its environment, names, where this build
        // registers it, or the swarm. See `design/avatars.md`.
        self.coordinator.choose(conversation.avatar());
        self.coordinator.update(
            &crate::avatar::Input::new(visual, anchor, dt)
                .appearing(appear)
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
        // In a space, the ray points at workers too, which only highlights them: the trigger still attracts the swarm, and holding X sends the worker pointed at with the words. See `design/interaction.md`. While the transcript shows, the ray is hidden and points at nothing.
        let in_space = focused
            && !in_arrival
            && !menu_shown
            && !wrist_active
            && conversation.joined_space().is_some();
        // In an environment that allows it, the left stick travels: aiming a teleport with the ray, landing as it returns to the middle, turning, stepping back and rising, each push owned by the direction it went first. The rig it moves applies from the next frame, as the platform places this frame's poses with it. See `design/locomotion.md`.
        let locomotion = conversation.locomotion();
        // Only once the environment's scenery has grown in, so that its surfaces are seen; and not while a card is carried, since aiming would put it down. A panel carried travels along, in the hand, as the rig carries the ray it follows.
        let grown = scenery.reveal.is_none() && scenery.environment == conversation.environment();
        let travel_active = in_space && grown && self.board.held().is_none();
        let travel = self.travel.update(locomotion::Input {
            place: place(conversation),
            locomotion: &locomotion,
            stick: self.travel_stick,
            ray: senses.ray,
            head: senses.head_pose.map(|pose| Vec3::from_array(pose.position)),
            facing: senses
                .head_pose
                .and_then(|pose| locomotion::facing(glam::Quat::from_array(pose.orientation))),
            active: travel_active,
            chosen: chosen_pad,
            dt,
        });
        // Landing clicks as choosing does; a pad chosen with A clicked as A chose it.
        if travel.teleported || (travel.transported && !pressed) {
            feedback.emit(feedback::Event::Choose, now);
        }
        // While a teleport is aimed, the ray serves only it, though a panel carried stays in the hand.
        let carrying_through = in_space;
        let in_space = in_space && !travel.aiming;
        // What is projected stands wherever the space shows it; the ray and the right grip act on it only while nothing else takes them, and not while a card is carried. A card the ray met nearer last frame takes the grip. See `design/projection.md`.
        conversation.observe_head(senses.head_pose.map(Into::into));
        // A change to a wall the server refused is drawn as the server has it at once, not as sent.
        if let Some(wall) = conversation.take_refused_wall() {
            self.projector.refused_wall(&wall);
        }
        // The teleport pad the ray meets, if any, which nothing nearer takes, below; a panel behind it is not pulled
        // off, as one behind a card is not.
        let pad_met = senses
            .ray
            .filter(|_| travel_active && !travel.aiming && !self.projector.carrying())
            .and_then(|ray| crate::pads::pick(&locomotion, &ray));
        let projected = self.projector.update(crate::projection::Input {
            projection: conversation.projection().filter(|_| !in_arrival),
            ray: senses.ray,
            // Only a participant pulls panels off; a visitor only looks at them and points.
            grip: senses
                .grip
                .filter(|_| self.board.held().is_none() && conversation.role().participates()),
            viewer: senses.viewer,
            active: carrying_through && self.board.held().is_none(),
            pointing: !travel.aiming,
            nearer: pad_met.map_or(self.card_reach, |(_, distance, _)| {
                self.card_reach.min(distance)
            }),
            stick: self.carry_stick,
            straighten: std::mem::take(&mut self.straighten),
            free: senses
                .trigger
                .is_some_and(|trigger| trigger > crate::pointing::PULL),
            controls: self.controls,
            scroll: scrolled(self.scroll_stick.as_ref(), self.wheel.take(), dt)
                .as_ref()
                .map(|(panel, rows)| (panel.as_str(), *rows)),
            rights: conversation.wall_rights(),
            now,
            dt,
        });
        if let Some(change) = projected.wall.clone() {
            feedback.emit(feedback::Event::Hover, now);
            conversation.set_wall(change);
        }
        if let Some(change) = projected.lock.clone() {
            feedback.emit(feedback::Event::Choose, now);
            conversation.lock_wall(change);
        }
        if projected.wall_ticked {
            feedback.emit(feedback::Event::Hover, now);
        }
        if projected.turned {
            feedback.emit(feedback::Event::Hover, now);
        }
        if projected.picked {
            feedback.emit(feedback::Event::Choose, now);
        }
        if let Some((panel, target)) = projected.placed.clone() {
            feedback.emit(feedback::Event::Hover, now);
            conversation.place_panel(panel, target);
        }
        let carrying = self.projector.carrying();
        // The space's cards stand wherever it shows them; the ray and the right grip act on them only while nothing else takes them, such as a panel carried. See `design/github.md`.
        let board = self.board.update(crate::board::Input {
            cards: conversation.cards().filter(|_| !in_arrival),
            ray: senses.ray,
            grip: senses.grip.filter(|_| !carrying),
            viewer: senses.viewer,
            active: in_space && !carrying,
            now,
        });
        self.card_reach = board
            .pointed
            .as_ref()
            .map_or(f32::INFINITY, |(_, distance, _)| *distance);
        if board.picked {
            feedback.emit(feedback::Event::Choose, now);
        }
        if let Some((card, placement)) = board.placed {
            feedback.emit(feedback::Event::Hover, now);
            conversation.place_card(card, placement);
        }
        let worker = self
            .delegation_scene
            .point(senses.ray.filter(|_| in_space))
            .map(str::to_owned);
        // The nearer of the card and the worker the ray meets is pointed at; a card carried is pointed at all along.
        let reach = |point: Vec3| {
            senses
                .ray
                .map_or(f32::INFINITY, |ray| point.distance(ray.origin))
        };
        let card = self.board.held().map(str::to_owned).or_else(|| {
            board
                .pointed
                .as_ref()
                .filter(|(_, distance, _)| {
                    self.delegation_scene
                        .hit_point()
                        .is_none_or(|point| *distance < reach(point))
                })
                .map(|(card, _, _)| card.clone())
        });
        let worker = worker.filter(|_| card.is_none());
        // Or the "+N more" sign of a cluster, which stands for the workers gathered into it.
        let gathered = self
            .delegation_scene
            .pointed_gathered()
            .filter(|_| card.is_none())
            .map(str::to_owned);
        // The environment's client side, in a space whose scenery has grown in, made for the space and environment it
        // is in. It points at what the environment shows only where nothing else is nearer, and not while a card is
        // carried.
        let shown = conversation.client_side().filter(|(environment, _)| {
            scenery.reveal.is_none() && scenery.environment == *environment
        });
        let space = conversation
            .joined_space()
            .map(|(space, _)| space.id.clone());
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
                        scrollable: None,
                        scroller: Default::default(),
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
        let nearest = card
            .as_ref()
            .and_then(|_| board.pointed.as_ref().map(|(_, distance, _)| *distance))
            .or_else(|| {
                worker
                    .as_ref()
                    .and(self.delegation_scene.hit_point())
                    .map(reach)
            })
            .unwrap_or(f32::INFINITY);
        let picked = self
            .client
            .as_ref()
            .zip(
                senses
                    .ray
                    .filter(|_| in_space && self.board.held().is_none()),
            )
            // The client side picks in the environment's own coordinates.
            .and_then(|(client, ray)| client.side.pick(&arrangement.carriage.unplace_ray(&ray)));
        let object = picked.clone().filter(|picked| picked.distance < nearest);
        let (card, worker) = if object.is_some() {
            (None, None)
        } else {
            (card, worker)
        };
        // A panel is pointed at where nothing else the ray meets is nearer; a panel carried is pointed at all along.
        let nearest = object.as_ref().map_or(nearest, |picked| picked.distance);
        let panel = self.projector.held().map(str::to_owned).or_else(|| {
            projected
                .pointed
                .as_ref()
                .filter(|(_, distance, _)| *distance < nearest && self.board.held().is_none())
                .map(|(panel, _, _)| panel.clone())
        });
        let (card, worker, object) = if panel.is_some() {
            (None, None, None)
        } else {
            (card, worker, object)
        };
        // A teleport pad is pointed at, while travel may be used and nothing is carried, where the ray meets nothing else
        // nearer that it may point at or take hold of: a card, a worker, an environment's object, or what the projector
        // meets: a panel pointed at or a wall's board and handles.
        let met = [
            board.pointed.as_ref().map(|(_, distance, _)| *distance),
            self.delegation_scene.hit_point().map(reach),
            picked.as_ref().map(|picked| picked.distance),
            projected.hit.map(|hit| hit.distance),
        ]
        .into_iter()
        .flatten()
        .fold(f32::INFINITY, f32::min);
        let pad = pad_met
            .filter(|_| !self.projector.carrying())
            .filter(|(_, distance, _)| *distance < met);
        let (card, worker, object, panel) = if pad.is_some() {
            (None, None, None, None)
        } else {
            (card, worker, object, panel)
        };
        // What the ray is on, Tomori's own interface first, which the trigger and the sticks act on next frame: a wall's
        // board, bar or handles, where the participant may take hold of walls, or a wall carried, count as Tomori's too.
        let on_wall = pad.is_none()
            && projected.hit.is_some_and(|hit| {
                hit.held
                    || picked
                        .as_ref()
                        .is_none_or(|picked| hit.distance < picked.distance)
            });
        self.aim = if menu_shown
            || on_wall
            || wrist_active
            || travel.aiming
            || self.highlighted.is_some()
            || panel.is_some()
            || card.is_some()
            || worker.is_some()
        {
            precedence::Aim::Tomori
        } else if pad.is_some() {
            precedence::Aim::Pad
        } else if object.is_some() {
            precedence::Aim::Environment
        } else {
            precedence::Aim::Nothing
        };
        // What the trigger and A choose next frame.
        self.pointed_pad = pad.map(|(key, _, _)| key);
        conversation.point_pad(pad.is_some());
        // What the trigger and B act on next frame.
        if let Some(client) = self.client.as_mut() {
            client.focus = object
                .as_ref()
                .filter(|picked| picked.selectable)
                .map(|picked| picked.object.clone());
            // What a push of the right stick takes next frame.
            client.scrollable = object
                .as_ref()
                .filter(|picked| picked.actions.contains(crate::client_side::Actions::SCROLL))
                .map(|picked| picked.object.clone());
        }
        // Entering a new target, an arrival object, a hand menu row, a card or a worker, ticks very lightly, but not while the trigger pulls the swarm.
        // While a teleport is aimed, only finding a landing ticks, even with a panel carried.
        let target = if travel.aiming {
            travel.landing.map(|_| feedback::key("landing"))
        } else {
            self.highlighted
                .as_ref()
                .map(feedback::key)
                .or_else(|| card.as_ref().map(feedback::key))
                .or_else(|| worker.as_ref().map(feedback::key))
                .or_else(|| object.as_ref().map(|picked| feedback::key(&picked.object)))
                .or_else(|| panel.as_ref().map(feedback::key))
                .or_else(|| pad.map(|(key, _, _)| feedback::key(&("pad", key))))
        };
        if self.hover.update(target, attracting) {
            feedback.emit(feedback::Event::Hover, now);
        }
        // The ray ends on what it meets, of every kind it may point at, the nearest, or on what is held: a card or a
        // panel carried, a wall carried or a handle dragged. See `design/interaction.md`.
        let ends = senses.ray.map(|ray| {
            let card_end = match self.board.held() {
                Some(_) => board
                    .end
                    .map(|end| crate::pointing::End::held(ray.origin, end)),
                None => board
                    .pointed
                    .as_ref()
                    .map(|(_, distance, point)| crate::pointing::End::new(*distance, *point)),
            };
            let object_end = picked
                .as_ref()
                .map(|picked| crate::pointing::End::new(picked.distance, ray.at(picked.distance)));
            let worker_end = self
                .delegation_scene
                .hit_point()
                .map(|point| crate::pointing::End::new(point.distance(ray.origin), point));
            let pad_end =
                pad.map(|(_, distance, point)| crate::pointing::End::new(distance, point));
            crate::pointing::end([card_end, projected.hit, object_end, worker_end, pad_end])
        });
        // The checklist item the ray meets on the panel pointed at, not one carried, which A ticks or unticks next.
        let item = match (&panel, &projected.pointed, self.projector.held()) {
            (Some(panel), Some((pointed, ..)), None) if pointed == panel => {
                projected.item.map(|line| (panel.clone(), line))
            }
            _ => None,
        };
        conversation.point_item(item.clone());
        // The label on the ray names the panel pointed at, not one carried, and says how to tick the item it meets.
        let mut projection = projected.sheets;
        if let (Some(panel), Some((_, _, point)), None) =
            (&panel, &projected.pointed, self.projector.held())
            && let Some(shown) = conversation
                .projection()
                .and_then(|projection| projection.panel(panel))
        {
            // A choosable row says how to choose it, or, just chosen, that it was; a checklist item how to tick it.
            let chosen = conversation
                .pointed_row()
                .filter(|_| conversation.ticks())
                .map(|(panel, row)| {
                    if conversation.just_chose(&panel, &row) {
                        crate::projection::ItemAction::Chosen
                    } else {
                        crate::projection::ItemAction::Choose
                    }
                });
            let tick = chosen
                .or_else(|| {
                    item.as_ref()
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
            projection.push(crate::projection::label(
                shown,
                *point,
                senses.viewer.unwrap_or(arrival::HOME),
                self.projector.scrollable(panel).then_some(self.controls),
                tick,
            ));
        }
        // The panel pointed at, not one carried, which the right stick and the wheel scroll next.
        self.pointed_panel = panel.clone().filter(|_| self.projector.held().is_none());
        // The label beside a teleport pad pointed at says where it goes.
        let pairs: Vec<_> = locomotion.pads().collect();
        projection.extend(pad.and_then(|(key, _, point)| {
            crate::pads::label(
                &pairs,
                key,
                point,
                senses.viewer.unwrap_or(arrival::HOME),
                self.controls,
            )
        }));
        // The hint beside a panel carried, over everything else.
        projection.extend(projected.hint);
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
        match (card, worker, &object) {
            // Pointed at, rather than carried, it names the list item the ray meets, if any.
            _ if panel.is_some() => {
                let item = projected
                    .pointed
                    .as_ref()
                    .filter(|(pointed, ..)| panel.as_deref() == Some(pointed.as_str()))
                    .and(projected.item);
                // A row of an environment's list is named by its id too, so that the agent is told of the row the
                // participant saw, even as the list changes.
                let row = panel
                    .as_deref()
                    .zip(item)
                    .and_then(|(panel, line)| conversation.row_at(panel, line))
                    .map(|row| row.id.clone());
                conversation.point(panel.clone().map(|panel| match (item, row) {
                    (Some(line), Some(row)) => {
                        tomori_protocol::Resource::panel_row(panel, line, row)
                    }
                    _ => tomori_protocol::Resource::panel_at(panel, item),
                }))
            }
            (Some(card), _, _) => conversation.point(Some(tomori_protocol::Resource::card(card))),
            (None, Some(worker), _) => {
                conversation.point(Some(tomori_protocol::Resource::worker(worker)))
            }
            (None, None, _) if gathered.is_some() => {
                conversation.point(gathered.map(tomori_protocol::Resource::gathered))
            }
            (None, None, Some(picked)) => {
                conversation.point_at_object(&picked.object, &picked.label)
            }
            (None, None, None) => conversation.point(None),
        }
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
                let push = match (&self.right_stick, &self.wheel_push) {
                    (RightStick::Object(object, number), _) => {
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
                let output = client.side.update(
                    crate::client_side::Input::new(updates, now)
                        .pointing(object.as_ref().map(|picked| picked.object.as_str()), head)
                        .with_select(client.select.take())
                        .with_back(std::mem::take(&mut client.back))
                        .with_sticks(left, right)
                        .with_scroll(scroll),
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
                output.drawn
            }
            None => None,
        };
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
        self.delegation_scene.arrange(
            arrangement.workers,
            arrangement.audience,
            arrangement.extent,
            arrangement.placement,
        );
        self.delegation_scene.update(root, workspace, dt);
        // The ray is drawn wherever it points at something: at arrival's objects or the hand menu, and in a space at the workers, where it ends among the worker it meets or else stops short of them. See `design/interaction.md`.
        let ray = senses.ray.filter(|_| senses.beam);
        let beam = if in_arrival || menu_shown {
            ray.zip(pointing.endpoint(crate::pointing::RAY_LENGTH))
                .map(|(ray, end)| crate::pointing::Beam {
                    origin: ray.origin,
                    end,
                    hit: pointing.hit.is_some(),
                })
        } else if travel.aiming {
            // Aiming a teleport, it ends at the landing, highlighted, or is plain and short where there is nowhere to land.
            ray.map(|ray| crate::pointing::Beam {
                origin: ray.origin,
                end: travel
                    .landing
                    .unwrap_or_else(|| ray.at(crate::pointing::SPACE_RAY_LENGTH)),
                hit: travel.landing.is_some(),
            })
        } else {
            let hit = ends.flatten().map(|end| end.point);
            ray.filter(|_| in_space).map(|ray| crate::pointing::Beam {
                origin: ray.origin,
                end: hit.unwrap_or_else(|| ray.at(crate::pointing::SPACE_RAY_LENGTH)),
                hit: hit.is_some(),
            })
        };
        // Arrival's objects are still drawn as they leave on entering a space.
        let arrival = (in_arrival
            || menu_shown
            || beam.is_some()
            || !scene.objects.is_empty()
            || scene.heading.is_some())
        .then(|| Arrival {
            hovered: pointing.hit.as_ref().map(|hit| hit.key.clone()),
            ray: beam,
            time: self.arrival_time,
            scene,
        });
        // Every source is heard from where it is, relative to the head; see `design/spatial-audio.md`.
        let agent = self.coordinator.voice();
        // The others in a space see this participant's head, controllers and hands, in the space's scene coordinates. Visitors send no presence; see `design/security.md`.
        let presence = senses
            .head_pose
            .filter(|_| conversation.role().participates())
            .map(|head| Presence {
                head,
                left: senses.controllers[crate::controller::LEFT],
                right: senses.controllers[crate::controller::RIGHT],
                hands: senses.hands,
                carrying: self.projector.held().map(str::to_owned),
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
        Frame {
            scenery,
            swarm: self.coordinator.swarm(),
            marker,
            attracting,
            card: self.presentation_navigation.card,
            content: self.presentation_navigation.content,
            cubes: self.delegation_scene.cubes(),
            stations: self.delegation_scene.stations(),
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
            landing: travel.landing,
            blink: travel.blink,
            pads: if in_space_shown && grown {
                crate::pads::shown(&locomotion, self.pointed_pad)
            } else {
                Vec::new()
            },
            environment: drawn,
            carriage,
            avatar: self.coordinator.shown(),
        }
    }
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
mod tests {
    use super::fixtures::*;
    use super::*;
    use crate::{conversation::LEAVE_HOLD, hand_menu::HOLD};
    use std::time::Duration;
    use tomori_client::{
        connection::Action,
        feedback::{Hand as Side, Haptic},
        playback::Earcon,
    };
    use tomori_protocol::ConnectionRole;

    const FRAME: f32 = 1.0 / 72.0;

    /// A head standing at home, looking ahead.
    fn senses() -> Senses {
        let head = Mat4::from_translation(arrival::HOME);
        Senses {
            focused: true,
            head: Some(head),
            head_pose: Pose::from_transform(head),
            viewer: Some(arrival::HOME),
            space_head: head,
            trigger: Some(0.0),
            ..Default::default()
        }
    }

    /// A right palm `degrees` from facing straight up, held in front of the head.
    fn palm_up(degrees: f32) -> Palm {
        let normal = glam::Quat::from_rotation_z(degrees.to_radians()) * Vec3::Y;
        Palm {
            wrist: arrival::HOME + Vec3::new(0.1, -0.35, -0.3),
            normal,
        }
    }

    /// A left palm `degrees` from facing straight up, held up in front of the head, as the right one in [`palm_up`] is.
    fn left_palm_up(degrees: f32) -> Palm {
        Palm {
            wrist: arrival::HOME + Vec3::new(-0.06, -0.2, -0.45),
            ..palm_up(degrees)
        }
    }

    /// A ray from the eye through the middle of `shape`.
    fn aim_at(shape: crate::pointing::Shape) -> Option<Ray> {
        Ray::new(arrival::HOME, shape.centre() - arrival::HOME)
    }

    /// The target with `key` in `scene`.
    fn target(scene: &arrival::Scene, key: &arrival::Key) -> crate::pointing::Target<arrival::Key> {
        scene
            .targets()
            .into_iter()
            .find(|target| &target.key == key)
            .expect("the target")
    }

    fn earcons(feedback: &mut Feedback) -> Vec<Earcon> {
        feedback.take_earcons().map(|(earcon, _)| earcon).collect()
    }

    fn felt_on(feedback: &mut Feedback, side: Side, now: Instant) -> Vec<Haptic> {
        feedback
            .haptics(now + Duration::from_secs(1))
            .into_iter()
            .filter(|(hand, _)| *hand == side)
            .map(|(_, haptic)| haptic)
            .collect()
    }

    fn press(change: impl FnOnce(&mut Buttons)) -> Buttons {
        let mut buttons = Buttons::default();
        change(&mut buttons);
        buttons
    }

    /// What a fake client side was told each frame: how many updates, what was pointed at, and whether the head was known.
    type Told = std::sync::Arc<std::sync::Mutex<Vec<(usize, Option<String>, bool)>>>;

    /// A client side that picks whatever the ray meets a metre away, asks and sounds once a frame, and shows how many
    /// frames it has had, keeping what it was told.
    struct Fake {
        frames: usize,
        told: Told,
    }

    impl crate::client_side::ClientSide for Fake {
        fn pick(&self, _ray: &Ray) -> Option<crate::client_side::Picked> {
            Some(crate::client_side::Picked::new("thing-1", "the thing", 1.0))
        }

        fn update(&mut self, input: crate::client_side::Input<'_>) -> crate::client_side::Output {
            self.frames += 1;
            self.told.lock().unwrap().push((
                input.updates.len(),
                input.pointed.map(str::to_owned),
                input.head.is_some(),
            ));
            crate::client_side::Output::new()
                .with_intent(tomori_protocol::Payload::new(serde_json::json!(
                    self.frames
                )))
                .with_sound(
                    crate::client_side::Sound::new(vec![0.0; 4], 48_000),
                    [0.0, 1.0, -1.0],
                )
                .drawing(std::sync::Arc::new(self.frames))
        }
    }

    #[test]
    fn an_environment_s_client_side_is_made_for_its_space_once_it_grows_in_and_presents_what_its_side_shares()
     {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::{Arc, Mutex};
        use tomori_client::Event;
        use tomori_protocol::Resource;
        use tomori_protocol::{ClientMessage, ServerMessage};
        let made = Arc::new(AtomicUsize::new(0));
        let told = Arc::new(Mutex::new(Vec::new()));
        let factory: crate::client_side::Factory = {
            let (made, told) = (made.clone(), told.clone());
            Arc::new(move |space: &str| {
                assert_eq!(space, SPACE);
                made.fetch_add(1, Ordering::Relaxed);
                Box::new(Fake {
                    frames: 0,
                    told: told.clone(),
                }) as Box<dyn crate::client_side::ClientSide>
            })
        };
        let mut conversation = lobby(ConnectionRole::Owner);
        conversation.set_client_sides(vec![(crate::test_support::SKY, factory)]);
        assert!(conversation.client_side().is_none());
        let mut step = Step::default();
        let mut feedback = felt();
        let now = Instant::now();
        let senses = Senses {
            ray: Ray::new(arrival::HOME, Vec3::NEG_Z),
            ..senses()
        };
        conversation.apply(space(|_| {}));
        conversation.take_actions();
        assert_eq!(
            conversation
                .client_side()
                .map(|(environment, _)| environment),
            Some(crate::test_support::SKY)
        );
        // Not while the space's scenery grows in.
        let growing = step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
        assert!(growing.environment.is_none());
        let mut shown = None;
        for _ in 0..400 {
            shown = step
                .update(&senses, &mut conversation, &mut feedback, now, FRAME)
                .environment;
        }
        // Made once, it shows what it shows, sounds, and points at what the ray meets, which the participant attends to.
        assert_eq!(made.load(Ordering::Relaxed), 1);
        let frames = *shown.unwrap().downcast_ref::<usize>().unwrap();
        assert!(frames > 1);
        assert_eq!(
            told.lock().unwrap().last(),
            Some(&(0, Some("thing-1".to_owned()), true))
        );
        assert_eq!(feedback.take_ambient().count(), 1);
        conversation.talk(true);
        conversation.talk(false);
        let actions = conversation.take_actions();
        let Some(Action::BeginUtterance(utterance)) = actions.first() else {
            unreachable!("recording begins, not {actions:?}")
        };
        assert_eq!(utterance.attention, [Resource::object("thing-1")]);
        // Before the server sends the environment's state, its intents are not sent; after, they are, and it is told.
        assert!(!format!("{actions:?}").contains("EnvironmentIntent"));
        conversation.apply(Event::Message(ServerMessage::EnvironmentState {
            environment: "sky".into(),
            version: 1,
            state: tomori_protocol::Payload::new(serde_json::json!({})),
        }));
        step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
        assert_eq!(told.lock().unwrap().last().unwrap().0, 1);
        assert!(conversation.take_actions().iter().any(|action| matches!(
            action,
            Action::Send(ClientMessage::EnvironmentIntent { .. })
        )));
        // Leaving, it is gone; joining again, it is made anew.
        conversation.apply(Event::Message(ServerMessage::LEFT));
        assert!(
            step.update(&senses, &mut conversation, &mut feedback, now, FRAME)
                .environment
                .is_none()
        );
        assert!(conversation.client_side().is_none());
        conversation.apply(space(|_| {}));
        for _ in 0..400 {
            step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
        }
        assert_eq!(made.load(Ordering::Relaxed), 2);
        // An environment it has no client side for has none.
        conversation.set_client_sides(Vec::new());
        assert!(conversation.client_side().is_none());
        assert!(
            step.update(&senses, &mut conversation, &mut feedback, now, FRAME)
                .environment
                .is_none()
        );
    }

    /// What a client side with a selectable object was told each frame: the trigger, B and the sticks.
    pub(super) type Done = std::sync::Arc<
        std::sync::Mutex<Vec<(Option<crate::client_side::Select>, bool, Vec2, Vec2)>>,
    >;

    /// A client side whose lamp, a metre along any ray ahead, is selectable, or not, as the farmyard's chickens are not,
    /// keeping what the participant did with it.
    struct Lamp {
        done: Done,
        selectable: bool,
        /// How far along the ray it is.
        distance: f32,
    }

    impl crate::client_side::ClientSide for Lamp {
        fn pick(&self, ray: &Ray) -> Option<crate::client_side::Picked> {
            let lamp = crate::client_side::Picked::new("lamp", "the lamp", self.distance);
            (ray.direction.z < 0.0).then(|| {
                if self.selectable {
                    lamp.selectable()
                } else {
                    lamp
                }
            })
        }

        fn update(&mut self, input: crate::client_side::Input<'_>) -> crate::client_side::Output {
            self.done.lock().unwrap().push((
                input.select,
                input.back,
                input.left_stick,
                input.right_stick,
            ));
            crate::client_side::Output::new()
        }
    }

    /// Joined to [`SPACE`] with a [`Lamp`], selectable or not, for Sky, where `locomotion` lets the participant travel,
    /// and what it is told.
    pub(super) fn lamp(
        locomotion: crate::locomotion::Locomotion,
        selectable: bool,
    ) -> (Conversation, Done) {
        lamp_at(locomotion, selectable, 1.0)
    }

    /// As [`lamp`], with the lamp `distance` along any ray ahead.
    fn lamp_at(
        locomotion: crate::locomotion::Locomotion,
        selectable: bool,
        distance: f32,
    ) -> (Conversation, Done) {
        let done = Done::default();
        let factory: crate::client_side::Factory = {
            let done = done.clone();
            std::sync::Arc::new(move |_: &str| {
                Box::new(Lamp {
                    done: done.clone(),
                    selectable,
                    distance,
                }) as Box<dyn crate::client_side::ClientSide>
            })
        };
        let mut conversation = lobby(ConnectionRole::Owner);
        conversation.set_client_sides(vec![(crate::test_support::SKY, factory)]);
        conversation.set_locomotion(vec![(crate::test_support::SKY, locomotion)]);
        conversation.apply(space(|_| {}));
        conversation.take_actions();
        (conversation, done)
    }

    /// `step` on until the space's scenery has grown in, as `senses` find it.
    fn grow(step: &mut Step, conversation: &mut Conversation, senses: &Senses, now: Instant) {
        for _ in 0..400 {
            step.update(senses, conversation, &mut felt(), now, FRAME);
        }
    }

    /// In [`SPACE`] with a selectable [`Lamp`] grown in, pointing at it, with what it is told and `locomotion` for Sky.
    fn lit(locomotion: crate::locomotion::Locomotion) -> (Conversation, Step, Senses, Done) {
        let (mut conversation, done) = lamp(locomotion, true);
        let mut step = Step::default();
        let senses = Senses {
            ray: Ray::new(arrival::HOME, Vec3::NEG_Z),
            ..senses()
        };
        grow(&mut step, &mut conversation, &senses, Instant::now());
        done.lock().unwrap().clear();
        (conversation, step, senses, done)
    }

    /// The selects the lamp was told since this was last asked.
    fn selects(done: &Done) -> Vec<crate::client_side::Select> {
        done.lock()
            .unwrap()
            .drain(..)
            .filter_map(|done| done.0)
            .collect()
    }

    #[test]
    fn a_pull_that_outlives_its_client_side_is_nobody_s_until_it_is_let_go() {
        use crate::client_side::Select;
        let now = Instant::now();
        let to = |environment: &'static str| {
            move |conversation: &mut Conversation| {
                conversation.apply(space(|state| state.environment = environment.into()));
            }
        };
        type Change = Box<dyn Fn(&mut Conversation)>;
        let changes: [Vec<Change>; 2] = [
            // The space moves to an environment with no client side, and back.
            vec![Box::new(to("minimal")), Box::new(to("sky"))],
            // Another space.
            vec![Box::new(|conversation: &mut Conversation| {
                conversation.apply(space(|state| state.id = "space-2".into()));
            })],
        ];
        for change in changes {
            let (mut conversation, mut step, mut senses, done) = lit(Default::default());
            senses.trigger = Some(0.3);
            assert!(
                !step
                    .update(&senses, &mut conversation, &mut felt(), now, FRAME)
                    .attracting
            );
            // Changed mid-pull, its client side goes and is made anew, and the pull, held hard, never attracts halfway,
            // nor selects the new one's lamp.
            senses.trigger = Some(1.0);
            for change in &change {
                change(&mut conversation);
                for _ in 0..400 {
                    let frame = step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
                    assert!(!frame.attracting);
                }
            }
            assert!(step.client.is_some());
            assert!(selects(&done).is_empty());
            // Let go, the next pull on the lamp is the new client side's.
            senses.trigger = Some(0.0);
            step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
            senses.trigger = Some(1.0);
            step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
            assert_eq!(selects(&done), [Select::Pressed("lamp".into())]);
        }
        // A pull begun before there was a client side stays nobody's once it is made, as it began, drawing nothing in
        // a space.
        let (mut conversation, done) = lamp(Default::default(), true);
        let mut step = Step::default();
        let senses = Senses {
            ray: Ray::new(arrival::HOME, Vec3::NEG_Z),
            trigger: Some(1.0),
            ..senses()
        };
        let mut attracting = Vec::new();
        for _ in 0..400 {
            attracting.push(
                step.update(&senses, &mut conversation, &mut felt(), now, FRAME)
                    .attracting,
            );
        }
        assert!(step.client.is_some());
        assert!(attracting.iter().all(|attracting| !*attracting));
        assert!(selects(&done).is_empty());
    }

    #[test]
    fn b_goes_to_the_first_rung_that_wants_it_and_to_the_environment_only_when_none_does() {
        use crate::conversation::Back;
        use tomori_client::Event;
        use tomori_protocol::{Panel, Place, QueueEntry, QueueState, ServerMessage, SpeechSetting};
        /// A projected panel 0.6 m ahead, nearer than the lamp, so that the ray points at it.
        fn panel(conversation: &mut Conversation) {
            let ahead = arrival::HOME + Vec3::new(0.0, 0.0, -0.6);
            fixtures::show_projection(
                conversation,
                Vec::new(),
                vec![Panel::new(
                    "panel-1",
                    "Build status",
                    "Green.",
                    Place::Free {
                        pose: tomori_protocol::Pose::upright_facing(
                            ahead.to_array(),
                            arrival::HOME.to_array(),
                        ),
                    },
                )],
            );
        }
        /// A wall of `kind` 0.6 m ahead, an empty slot of its top row where the ray meets it, nearer than the lamp, taken
        /// hold of with the grip: carried, if someone added it, or else held, its handles showing.
        fn wall(
            conversation: &mut Conversation,
            step: &mut Step,
            senses: &mut Senses,
            now: Instant,
            kind: tomori_protocol::WallKind,
        ) {
            let middle = arrival::HOME + Vec3::new(0.0, -0.3, -0.6);
            fixtures::show_editable_walls(
                conversation,
                vec![tomori_protocol::Wall::new(
                    "wall-1",
                    kind,
                    tomori_protocol::Pose::upright_facing(
                        middle.to_array(),
                        [arrival::HOME.x, middle.y, arrival::HOME.z],
                    ),
                )],
                Vec::new(),
            );
            grow(step, conversation, senses, now);
            senses.grip = Some(1.0);
            step.update(senses, conversation, &mut felt(), now, FRAME);
            if kind != tomori_protocol::WallKind::Added {
                senses.grip = Some(0.0);
                step.update(senses, conversation, &mut felt(), now, FRAME);
            }
            assert_eq!(step.projector.editing_wall(), Some("wall-1"));
        }
        fn failed(conversation: &mut Conversation, error: &str) {
            conversation.apply(Event::Message(ServerMessage::TranscriptFailed {
                utterance: "utterance-9".into(),
                error: error.into(),
            }));
        }
        type Setup = fn(&mut Conversation, &mut Step, &mut Senses, Instant);
        let rows: [(Back, Setup); 14] = [
            (Back::Carried, |conversation, step, senses, now| {
                panel(conversation);
                senses.grip = Some(1.0);
                grow(step, conversation, senses, now);
                assert!(step.projector.held().is_some());
            }),
            (Back::CarriedWall, |conversation, step, senses, now| {
                wall(
                    conversation,
                    step,
                    senses,
                    now,
                    tomori_protocol::WallKind::Added,
                );
                assert_eq!(step.projector.carried_wall(), Some("wall-1"));
            }),
            (Back::HeldWall, |conversation, step, senses, now| {
                wall(
                    conversation,
                    step,
                    senses,
                    now,
                    tomori_protocol::WallKind::Environment,
                );
                assert_eq!(step.projector.carried_wall(), None);
            }),
            (Back::Pinned, |conversation, step, _, now| {
                step.buttons(
                    &press(|buttons| buttons.y = true),
                    true,
                    conversation,
                    &mut felt(),
                    now,
                );
                assert!(step.pinned());
            }),
            // Showing, with something to back out of: here a problem, which it dismisses ahead of the space.
            (Back::Wrist, |conversation, step, senses, now| {
                senses.right_palm = Some(palm_up(0.0));
                wrist_after(step, conversation, senses, now);
                assert!(step.wrist.state().active());
                failed(conversation, "The recogniser failed.");
            }),
            (Back::Modes, |conversation, _, _, _| {
                conversation.apply(space(|state| state.speech = SpeechSetting::Full));
                conversation.refresh();
            }),
            (Back::Queued, |conversation, _, _, _| {
                conversation.apply(space(|state| {
                    state.queue = vec![QueueEntry {
                        id: "entry-1".into(),
                        source: 1,
                        speaker: "Ada".into(),
                        text: "Fix the parser.".into(),
                        state: QueueState::Queued,
                    }]
                }));
                conversation.select_queued(1);
            }),
            (Back::Naming, |conversation, _, _, _| {
                conversation.apply(Event::Message(ServerMessage::Name { name: "Ada".into() }));
                conversation.choose_menu(crate::hand_menu::Choice::Rename);
            }),
            (Back::Unheard, |conversation, _, _, _| {
                failed(conversation, tomori_protocol::NO_SPEECH)
            }),
            (Back::Problem, |conversation, _, _, _| {
                failed(conversation, "The recogniser failed.")
            }),
            (Back::SpaceProblem, |conversation, _, _, _| {
                conversation.apply(space(|state| {
                    state.error = Some("The agent stopped.".into())
                }));
            }),
            (Back::Panel, |conversation, step, senses, now| {
                panel(conversation);
                grow(step, conversation, senses, now);
            }),
            (Back::Environment, |_, _, _, _| {}),
            (Back::Nothing, |conversation, step, senses, now| {
                senses.ray = Ray::new(arrival::HOME, Vec3::Z);
                step.update(senses, conversation, &mut felt(), now, FRAME);
            }),
        ];
        for (rung, setup) in rows {
            let (mut conversation, mut step, mut senses, done) = lit(Default::default());
            let now = Instant::now();
            setup(&mut conversation, &mut step, &mut senses, now);
            step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
            // The rungs the conversation holds take B even while the lamp is pointed at.
            let pointed = step.client.as_ref().unwrap().focus.is_some();
            if matches!(
                rung,
                Back::Modes
                    | Back::Queued
                    | Back::Naming
                    | Back::Unheard
                    | Back::Problem
                    | Back::SpaceProblem
                    | Back::Environment
            ) {
                assert!(pointed, "{rung:?}");
            }
            done.lock().unwrap().clear();
            let b = |step: &mut Step, conversation: &mut Conversation, pressed, at| {
                step.buttons(
                    &press(|buttons| {
                        buttons.back = pressed;
                        buttons.back_held = true;
                    }),
                    true,
                    conversation,
                    &mut felt(),
                    at,
                );
            };
            b(&mut step, &mut conversation, true, now);
            step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
            assert_eq!(step.last_back, Some(rung));
            let told = done.lock().unwrap().iter().filter(|done| done.1).count();
            assert_eq!(told, usize::from(rung == Back::Environment), "{rung:?}");
            // Held, it leaves after a second, except on a panel or a wall carried, where it never does.
            b(&mut step, &mut conversation, false, now + LEAVE_HOLD / 2);
            assert!(conversation.joined_space().is_some(), "{rung:?}");
            b(&mut step, &mut conversation, false, now + LEAVE_HOLD);
            let left = conversation
                .take_actions()
                .contains(&Action::Send(tomori_protocol::ClientMessage::Leave));
            assert_eq!(
                left,
                !matches!(rung, Back::Carried | Back::CarriedWall),
                "{rung:?}"
            );
        }
        // The rows are in the order `Back` declares its rungs, which is B's order.
        assert!(rows.windows(2).all(|pair| pair[0].0 < pair[1].0));
        // Any two rungs at once, the later set up first, as what was open before: the earlier takes B, and the
        // environment is told nothing. Nothing is what is left when no rung wants B, so it pairs with none; and a panel
        // carried is the panel pointed at, is not taken up while the hand menu is pinned or the transcript shows, and
        // keeps the transcript away, so it pairs with none of those.
        for (index, (first, before)) in rows.iter().enumerate() {
            for (then, after) in &rows[index + 1..] {
                // A wall carried is let go as a panel carried is; no wall is taken hold of while the hand menu is
                // pinned or the transcript shows, as they take the ray; and each shows its own projection, which a
                // panel's replaces.
                if *then == Back::Nothing
                    || (matches!(first, Back::Carried | Back::CarriedWall)
                        && matches!(
                            then,
                            Back::CarriedWall
                                | Back::HeldWall
                                | Back::Pinned
                                | Back::Wrist
                                | Back::Panel
                        ))
                    || (*first == Back::HeldWall
                        && matches!(then, Back::Pinned | Back::Wrist | Back::Panel))
                {
                    continue;
                }
                let (mut conversation, mut step, mut senses, done) = lit(Default::default());
                let now = Instant::now();
                after(&mut conversation, &mut step, &mut senses, now);
                before(&mut conversation, &mut step, &mut senses, now);
                step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
                done.lock().unwrap().clear();
                step.buttons(
                    &press(|buttons| buttons.back = true),
                    true,
                    &mut conversation,
                    &mut felt(),
                    now,
                );
                step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
                assert_eq!(step.last_back, Some(*first), "{first:?} before {then:?}");
                let told = done.lock().unwrap().iter().filter(|done| done.1).count();
                assert_eq!(told, 0, "{first:?} before {then:?}");
            }
        }
    }

    #[test]
    fn a_trigger_pull_on_a_selectable_object_is_the_environment_s_and_never_attracts() {
        use crate::client_side::Select;
        let (mut conversation, mut step, mut senses, done) = lit(Default::default());
        let now = Instant::now();
        let mut pulls = Vec::new();
        for trigger in [0.3, 1.0, 1.0, 0.0] {
            senses.trigger = Some(trigger);
            let frame = step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
            pulls.push(frame.attracting);
        }
        assert_eq!(pulls, [false; 4], "a pull on the lamp never attracts");
        let selects: Vec<_> = done
            .lock()
            .unwrap()
            .drain(..)
            .filter_map(|done| done.0)
            .collect();
        assert_eq!(
            selects,
            [
                Select::Pressed("lamp".into()),
                Select::Released("lamp".into())
            ]
        );
        // Pointing at nothing of the environment's, a pull selects nothing, and in a space draws the swarm nowhere.
        senses.ray = Ray::new(arrival::HOME, Vec3::Z);
        step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        senses.trigger = Some(1.0);
        let frame = step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        assert!(!frame.attracting);
        assert!(done.lock().unwrap().iter().all(|done| done.0.is_none()));
    }

    #[test]
    fn b_is_the_environment_s_only_while_its_object_is_pointed_at_and_nothing_else_wants_it() {
        let (mut conversation, mut step, senses, done) = lit(Default::default());
        let now = Instant::now();
        let mut feedback = felt();
        let mut b = |step: &mut Step, conversation: &mut Conversation, pressed, held, at| {
            step.buttons(
                &press(|buttons| {
                    buttons.back = pressed;
                    buttons.back_held = held;
                }),
                true,
                conversation,
                &mut feedback,
                at,
            );
        };
        let backs = |done: &Done| done.lock().unwrap().drain(..).filter(|done| done.1).count();
        // Pointed at, B is the lamp's, once, without the notice that holding it leaves.
        b(&mut step, &mut conversation, true, false, now);
        step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        assert_eq!(backs(&done), 1);
        assert_eq!(conversation.notice(), None);
        // A problem shown takes B first.
        conversation.apply(space(|state| {
            state.error = Some("The agent stopped.".into())
        }));
        b(&mut step, &mut conversation, true, false, now);
        step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        assert_eq!(backs(&done), 0);
        // Losing focus forgets a press not yet told.
        b(&mut step, &mut conversation, true, false, now);
        step.blur(&mut conversation);
        step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        assert_eq!(backs(&done), 0);
        // Held, it still leaves, after a second, as ever.
        let later = now + Duration::from_secs(5);
        b(&mut step, &mut conversation, true, true, later);
        step.update(&senses, &mut conversation, &mut felt(), later, FRAME);
        assert_eq!(backs(&done), 1);
        b(
            &mut step,
            &mut conversation,
            false,
            true,
            later + LEAVE_HOLD / 2,
        );
        assert!(conversation.joined_space().is_some());
        b(
            &mut step,
            &mut conversation,
            false,
            true,
            later + LEAVE_HOLD,
        );
        assert!(
            conversation
                .take_actions()
                .contains(&Action::Send(tomori_protocol::ClientMessage::Leave))
        );
    }

    #[test]
    fn b_pointing_elsewhere_keeps_its_meaning() {
        let (mut conversation, mut step, mut senses, done) = lit(Default::default());
        let now = Instant::now();
        senses.ray = Ray::new(arrival::HOME, Vec3::Z);
        step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        step.buttons(
            &press(|buttons| buttons.back = true),
            true,
            &mut conversation,
            &mut felt(),
            now,
        );
        step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        assert!(done.lock().unwrap().iter().all(|done| !done.1));
        assert_eq!(conversation.notice(), Some("Hold B to leave the space."));
    }

    /// The press and hold of A, as the controller reports it.
    fn a_down() -> Buttons {
        press(|buttons| {
            buttons.a = true;
            buttons.a_held = true;
        })
    }

    /// A held, no new press.
    fn a_held() -> Buttons {
        press(|buttons| buttons.a_held = true)
    }

    /// What A is heard to do, by the choosing cue, of a frame step with `feedback`.
    fn chose(feedback: &mut Feedback) -> bool {
        earcons(feedback).contains(&Earcon::Choose)
    }

    #[test]
    fn a_goes_to_the_first_rung_that_wants_it_and_to_the_environment_only_when_none_does() {
        use crate::hand_menu::MenuKey;
        use precedence::A;
        use tomori_protocol::{Approval, ApprovalOption, QueueEntry, QueueState, SpeechSetting};
        /// Whether what A would do, were it the rung's, was done.
        type Effect = Box<dyn Fn(&mut Conversation, &mut Feedback) -> bool>;
        type Setup = fn(&mut Conversation, &mut Step, &mut Senses, Instant) -> Effect;
        /// What is set up after the last frame before A, which a frame would undo.
        type Late = fn(&mut Conversation);
        fn nothing(_: &mut Conversation) {}
        fn request(conversation: &mut Conversation) {
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
        }
        /// The hand menu pinned with Y and shown, with the ray on the Mute row, or beside every row.
        fn menu(
            conversation: &mut Conversation,
            step: &mut Step,
            senses: &mut Senses,
            now: Instant,
            row: bool,
        ) {
            step.buttons(
                &press(|buttons| buttons.y = true),
                true,
                conversation,
                &mut felt(),
                now,
            );
            senses.left_palm = Some(left_palm_up(0.0));
            let scene = step
                .update(senses, conversation, &mut felt(), now, FRAME)
                .arrival
                .expect("the pinned menu")
                .scene;
            if row {
                let mute = arrival::Key::Menu(MenuKey::Mute);
                senses.ray = aim_at(target(&scene, &mute).shape);
                senses.beam = true;
                step.update(senses, conversation, &mut felt(), now, FRAME);
                assert_eq!(step.highlighted, Some(mute));
            }
        }
        /// The participant pointing at the queue's choosable row `pod112`, as the frame before found it.
        fn on_row(conversation: &mut Conversation) {
            conversation.point_item(Some(("panel-2".into(), 1)));
        }
        /// Whether A chose the row pointed at, which the environment would be told.
        fn chose_row(actions: &[Action]) -> bool {
            actions.contains(&Action::Send(
                tomori_protocol::ClientMessage::ChoosePanelRow {
                    panel: "panel-2".into(),
                    row: "pod112".into(),
                    revision: Some(6),
                },
            ))
        }
        /// The queue floating behind the lamp, which the ray meets first.
        fn queue(conversation: &mut Conversation) {
            show_queue(
                conversation,
                arrival::HOME + Vec3::new(0.0, 0.0, -2.0),
                arrival::HOME,
            );
        }
        let rows: [(A, &str, Setup, Late); 18] = [
            (
                A::MenuRow,
                "a hand menu row",
                |conversation, step, senses, now| {
                    menu(conversation, step, senses, now, true);
                    let muted = conversation.menu_model().muted;
                    Box::new(move |conversation, _| conversation.menu_model().muted != muted)
                },
                nothing,
            ),
            (
                A::Menu,
                "the hand menu, off its rows",
                |conversation, step, senses, now| {
                    // A request waiting, which A would decide were it not for the menu, which A chooses only in.
                    request(conversation);
                    conversation.settle_request();
                    menu(conversation, step, senses, now, false);
                    assert!(step.highlighted.is_none());
                    Box::new(|conversation, _| {
                        conversation.take_actions().is_empty() && !conversation.a_refused()
                    })
                },
                nothing,
            ),
            (
                A::Transcript,
                "the transcript",
                |conversation, step, senses, now| {
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
                    senses.right_palm = Some(palm_up(0.0));
                    wrist_after(step, conversation, senses, now);
                    assert!(step.wrist.state().active());
                    // The right stick moves the focus onto a turn, which A then expands, heard.
                    step.buttons(
                        &press(|buttons| buttons.right_stick = Vec2::new(0.0, 1.0)),
                        true,
                        conversation,
                        &mut felt(),
                        now,
                    );
                    step.buttons(&Buttons::default(), true, conversation, &mut felt(), now);
                    Box::new(|_, feedback| chose(feedback))
                },
                nothing,
            ),
            (
                A::Conversation,
                "the list of modes",
                |conversation, _, _, _| {
                    conversation.apply(space(|state| state.speech = SpeechSetting::Full));
                    conversation.refresh();
                    Box::new(|conversation, feedback| {
                        chose(feedback)
                            && conversation.take_actions().contains(&Action::Send(
                                tomori_protocol::ClientMessage::SetSpeech {
                                    speech: SpeechSetting::Brief,
                                },
                            ))
                    })
                },
                nothing,
            ),
            (
                A::Conversation,
                "a request that has just appeared",
                |conversation, _, _, _| {
                    request(conversation);
                    Box::new(|conversation, feedback| {
                        conversation.a_refused()
                            && conversation.notice() == Some("Read the request before choosing.")
                            && !chose(feedback)
                            && conversation.take_actions().is_empty()
                    })
                },
                nothing,
            ),
            (
                A::Conversation,
                "a request that has just appeared, before a choosable row",
                |conversation, _, _, _| {
                    queue(conversation);
                    request(conversation);
                    Box::new(|conversation, feedback| {
                        conversation.a_refused()
                            && !chose(feedback)
                            && !chose_row(&conversation.take_actions())
                    })
                },
                on_row,
            ),
            (
                A::Conversation,
                "a checklist item pointed at",
                |conversation, _, _, _| {
                    // Behind the lamp, which the ray meets first.
                    let behind = arrival::HOME + Vec3::new(0.0, 0.0, -2.0);
                    show_checklist(conversation, behind, arrival::HOME);
                    Box::new(|conversation, feedback| {
                        chose(feedback)
                            && conversation.take_actions()
                                == [Action::Send(
                                    tomori_protocol::ClientMessage::CheckPanelItem {
                                        panel: "panel-1".into(),
                                        line: 2,
                                        checked: false,
                                        revision: Some(4),
                                    },
                                )]
                    })
                },
                |conversation| conversation.point_item(Some(("panel-1".into(), 2))),
            ),
            (
                A::Conversation,
                "a request to turn the right wrist to decide",
                |conversation, _, _, _| {
                    conversation.decide_on_wrist();
                    request(conversation);
                    conversation.settle_request();
                    Box::new(|conversation, feedback| {
                        conversation.a_refused()
                            && conversation.notice() == Some("Turn your right wrist to decide.")
                            && !chose(feedback)
                            && conversation.take_actions().is_empty()
                    })
                },
                nothing,
            ),
            (
                A::Conversation,
                "a request to decide",
                |conversation, _, _, _| {
                    request(conversation);
                    conversation.settle_request();
                    Box::new(|conversation, feedback| {
                        chose(feedback)
                            && conversation.take_actions()
                                == [Action::Send(tomori_protocol::ClientMessage::Decide {
                                    request: "request-1".into(),
                                    decision: tomori_protocol::Decision::AllowOnce,
                                    option: None,
                                })]
                    })
                },
                nothing,
            ),
            (
                A::Conversation,
                "a request to decide, before a choosable row",
                |conversation, _, _, _| {
                    queue(conversation);
                    request(conversation);
                    conversation.settle_request();
                    Box::new(|conversation, feedback| {
                        let actions = conversation.take_actions();
                        chose(feedback)
                            && actions
                                == [Action::Send(tomori_protocol::ClientMessage::Decide {
                                    request: "request-1".into(),
                                    decision: tomori_protocol::Decision::AllowOnce,
                                    option: None,
                                })]
                    })
                },
                on_row,
            ),
            (
                A::Conversation,
                "a question waiting, which refuses a choosable row",
                |conversation, _, _, _| {
                    queue(conversation);
                    conversation.apply(space(|state| {
                        state.projection = true;
                        state.choices = true;
                        state.question = Some(tomori_protocol::Question {
                            id: "question-1".into(),
                            title: "Which?".into(),
                            text: "Name it".into(),
                            field: "field-1".into(),
                            supported: true,
                        });
                    }));
                    conversation.settle_request();
                    conversation.take_actions();
                    Box::new(|conversation, feedback| {
                        conversation.a_refused()
                            && conversation.notice() == Some("Answer the request before choosing.")
                            && !chose(feedback)
                            && conversation.take_actions().is_empty()
                    })
                },
                on_row,
            ),
            (
                A::Conversation,
                "a selected queue entry",
                |conversation, _, _, _| {
                    conversation.apply(space(|state| {
                        state.queue = vec![QueueEntry {
                            id: "entry-1".into(),
                            source: 1,
                            speaker: "Ada".into(),
                            text: "Fix the parser.".into(),
                            state: QueueState::Queued,
                        }]
                    }));
                    conversation.select_queued(1);
                    Box::new(|conversation, feedback| {
                        chose(feedback) && conversation.text().contains("cancelling, A undoes")
                    })
                },
                nothing,
            ),
            (
                A::Conversation,
                "a selected queue entry, before a choosable row",
                |conversation, _, _, _| {
                    queue(conversation);
                    conversation.apply(space(|state| {
                        state.projection = true;
                        state.choices = true;
                        state.queue = vec![QueueEntry {
                            id: "entry-1".into(),
                            source: 1,
                            speaker: "Ada".into(),
                            text: "Fix the parser.".into(),
                            state: QueueState::Queued,
                        }]
                    }));
                    conversation.select_queued(1);
                    conversation.take_actions();
                    Box::new(|conversation, feedback| {
                        chose(feedback)
                            && conversation.text().contains("cancelling, A undoes")
                            && !chose_row(&conversation.take_actions())
                    })
                },
                on_row,
            ),
            (
                A::Conversation,
                "a choosable row of an environment's list pointed at, after everything that decides",
                |conversation, _, _, _| {
                    queue(conversation);
                    Box::new(|conversation, feedback| {
                        chose(feedback)
                            && conversation.take_actions()
                                == [Action::Send(
                                    tomori_protocol::ClientMessage::ChoosePanelRow {
                                        panel: "panel-2".into(),
                                        row: "pod112".into(),
                                        revision: Some(6),
                                    },
                                )]
                    })
                },
                on_row,
            ),
            (
                A::Conversation,
                "a teleport pad pointed at, the last of its meanings",
                |conversation, _, senses, _| {
                    // Behind the head, where the lamp, which is ahead, does not pick; the pad takes A there, and, as
                    // the frame before found it, would do before the lamp were it still in focus.
                    conversation
                        .set_locomotion(vec![(crate::test_support::SKY, pad_behind_home())]);
                    senses.ray = Ray::new(arrival::HOME, Vec3::new(0.0, 0.3, 2.0) - arrival::HOME);
                    Box::new(|conversation, feedback| {
                        chose(feedback) && conversation.take_actions().is_empty()
                    })
                },
                nothing,
            ),
            (
                A::Tomori,
                "a wall in front of the lamp",
                |conversation, step, senses, now| {
                    let middle = arrival::HOME + Vec3::new(0.0, -0.3, -0.6);
                    fixtures::show_editable_walls(
                        conversation,
                        vec![tomori_protocol::Wall::new(
                            "wall-1",
                            tomori_protocol::WallKind::Added,
                            tomori_protocol::Pose::upright_facing(
                                middle.to_array(),
                                [arrival::HOME.x, middle.y, arrival::HOME.z],
                            ),
                        )],
                        Vec::new(),
                    );
                    grow(step, conversation, senses, now);
                    Box::new(|conversation, feedback| {
                        conversation.take_actions().is_empty() && !chose(feedback)
                    })
                },
                nothing,
            ),
            (
                A::Environment,
                "the lamp, with nothing before it",
                |_, _, _, _| Box::new(|_, feedback| !chose(feedback)),
                nothing,
            ),
            (
                A::Nothing,
                "the ray on nothing",
                |conversation, step, senses, now| {
                    senses.ray = Ray::new(arrival::HOME, Vec3::Z);
                    step.update(senses, conversation, &mut felt(), now, FRAME);
                    Box::new(|conversation, feedback| {
                        conversation.take_actions().is_empty() && !chose(feedback)
                    })
                },
                nothing,
            ),
        ];
        // The rows are in the order `A` declares its rungs, which is A's order.
        let position = |rung: A| A::ORDER.iter().position(|other| *other == rung).unwrap();
        assert!(
            rows.windows(2)
                .all(|pair| position(pair[0].0) <= position(pair[1].0))
        );
        for rung in A::ORDER {
            assert!(rows.iter().any(|row| row.0 == *rung), "{rung:?}");
        }
        for (rung, label, setup, late) in rows {
            let (mut conversation, mut step, mut senses, done) = lit(Default::default());
            let now = Instant::now();
            let effect = setup(&mut conversation, &mut step, &mut senses, now);
            step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
            late(&mut conversation);
            let focus = |step: &Step| step.client.as_ref().unwrap().focus.clone();
            // The rungs of the frame step take A while the ray is not the environment's, so the lamp is not in focus
            // there; forcing it in shows that A goes first to them, as it would on the frame the menu or the
            // transcript appears, before the frame step has found what is pointed at. Everywhere else the lamp is
            // pointed at, as the frame before found it.
            if matches!(rung, A::MenuRow | A::Menu | A::Transcript) || step.pointed_pad.is_some() {
                step.client.as_mut().unwrap().focus = Some("lamp".into());
            }
            assert_eq!(
                focus(&step).as_deref(),
                (rung != A::Nothing).then_some("lamp"),
                "{label}"
            );
            assert_eq!(
                step.aim == precedence::Aim::Tomori,
                matches!(rung, A::MenuRow | A::Menu | A::Transcript | A::Tomori),
                "{label}"
            );
            done.lock().unwrap().clear();
            // Pressed and held, A goes to the rung, and to the lamp only if no rung before it wants it.
            let mut feedback = felt();
            step.buttons(&a_down(), true, &mut conversation, &mut feedback, now);
            step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
            let wanted = effect(&mut conversation, &mut feedback);
            assert!(wanted, "{label}");
            // The environment's own is told that the lamp was pressed and, once A is let go, that it was let go.
            assert_eq!(
                selects(&done),
                if rung == A::Environment {
                    vec![crate::client_side::Select::Pressed("lamp".into())]
                } else {
                    Vec::new()
                },
                "{label}"
            );
            step.buttons(&a_held(), true, &mut conversation, &mut felt(), now);
            step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
            assert!(selects(&done).is_empty(), "{label}");
            step.buttons(
                &Buttons::default(),
                true,
                &mut conversation,
                &mut felt(),
                now,
            );
            step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
            assert_eq!(
                selects(&done),
                if rung == A::Environment {
                    vec![crate::client_side::Select::Released("lamp".into())]
                } else {
                    Vec::new()
                },
                "{label}"
            );
        }
    }

    #[test]
    fn a_selects_only_a_selectable_object_pointed_at_and_only_with_input() {
        use crate::client_side::Select;
        let now = Instant::now();
        // An object that is not selectable, as the farmyard's chickens are not, is not selected by A.
        let (mut conversation, done) = lamp(Default::default(), false);
        let mut step = Step::default();
        let senses = Senses {
            ray: Ray::new(arrival::HOME, Vec3::NEG_Z),
            ..senses()
        };
        grow(&mut step, &mut conversation, &senses, now);
        assert_eq!(step.aim, precedence::Aim::Environment);
        step.buttons(&a_down(), true, &mut conversation, &mut felt(), now);
        step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        assert!(selects(&done).is_empty());
        // Without a client side, A does what it always did, and nothing is told.
        let mut conversation = joined();
        let mut step = Step::default();
        grow(&mut step, &mut conversation, &senses, now);
        assert!(step.client.is_none());
        step.buttons(&a_down(), true, &mut conversation, &mut felt(), now);
        step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        assert_eq!(step.a_press.on, None);
        // Without focus, a press selects nothing, and one held is let go.
        let (mut conversation, mut step, senses, done) = lit(Default::default());
        step.buttons(&a_down(), false, &mut conversation, &mut felt(), now);
        step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        assert!(selects(&done).is_empty());
        step.buttons(&a_down(), true, &mut conversation, &mut felt(), now);
        step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        assert_eq!(selects(&done), [Select::Pressed("lamp".into())]);
        step.buttons(&a_held(), false, &mut conversation, &mut felt(), now);
        step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        assert_eq!(selects(&done), [Select::Released("lamp".into())]);
        // Losing focus, as the platform reports it, lets go once, whether or not buttons are read between.
        step.buttons(&a_down(), true, &mut conversation, &mut felt(), now);
        step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        assert_eq!(selects(&done), [Select::Pressed("lamp".into())]);
        step.blur(&mut conversation);
        step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        assert_eq!(selects(&done), [Select::Released("lamp".into())]);
        step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        assert!(selects(&done).is_empty());
        // Pressed and let go within a frame, it is let go on the next.
        let click = press(|buttons| buttons.a = true);
        step.buttons(&click, true, &mut conversation, &mut felt(), now);
        step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        assert_eq!(selects(&done), [Select::Pressed("lamp".into())]);
        step.buttons(
            &Buttons::default(),
            true,
            &mut conversation,
            &mut felt(),
            now,
        );
        step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        assert_eq!(selects(&done), [Select::Released("lamp".into())]);
    }

    #[test]
    fn a_never_draws_the_swarm_and_the_trigger_and_a_select_one_at_a_time() {
        use crate::client_side::Select;
        let now = Instant::now();
        let lamp = || "lamp".to_owned();
        let frame = |step: &mut Step,
                     conversation: &mut Conversation,
                     senses: &Senses,
                     buttons: Buttons| {
            step.buttons(&buttons, true, conversation, &mut felt(), now);
            step.update(senses, conversation, &mut felt(), now, FRAME)
                .attracting
        };
        // A pull of the trigger pressed on the lamp keeps the select: a press of A is ignored, and letting go of it,
        // or of the pull, tells nothing more than the pull's own release.
        let (mut conversation, mut step, mut senses, done) = lit(Default::default());
        senses.trigger = Some(1.0);
        assert!(!frame(
            &mut step,
            &mut conversation,
            &senses,
            Buttons::default()
        ));
        assert_eq!(selects(&done), [Select::Pressed(lamp())]);
        assert!(!frame(&mut step, &mut conversation, &senses, a_down()));
        assert!(!frame(&mut step, &mut conversation, &senses, a_held()));
        assert!(!frame(
            &mut step,
            &mut conversation,
            &senses,
            Buttons::default()
        ));
        assert!(selects(&done).is_empty());
        senses.trigger = Some(0.0);
        frame(&mut step, &mut conversation, &senses, a_down());
        assert_eq!(selects(&done), [Select::Released(lamp())]);
        frame(&mut step, &mut conversation, &senses, Buttons::default());
        assert!(selects(&done).is_empty());
        // A press of A on the lamp keeps it: a pull of the trigger that begins meanwhile is nobody's, so that it
        // selects nothing, and, begun on the lamp, never attracts the swarm either, even once A is let go.
        senses.trigger = Some(0.0);
        frame(&mut step, &mut conversation, &senses, a_down());
        assert_eq!(selects(&done), [Select::Pressed(lamp())]);
        senses.trigger = Some(1.0);
        assert!(!frame(&mut step, &mut conversation, &senses, a_held()));
        assert!(!frame(
            &mut step,
            &mut conversation,
            &senses,
            Buttons::default()
        ));
        assert_eq!(selects(&done), [Select::Released(lamp())]);
        assert!(!frame(
            &mut step,
            &mut conversation,
            &senses,
            Buttons::default()
        ));
        senses.trigger = Some(0.0);
        frame(&mut step, &mut conversation, &senses, Buttons::default());
        senses.trigger = Some(1.0);
        frame(&mut step, &mut conversation, &senses, Buttons::default());
        assert_eq!(selects(&done), [Select::Pressed(lamp())]);
        senses.trigger = Some(0.0);
        frame(&mut step, &mut conversation, &senses, Buttons::default());
        assert_eq!(selects(&done), [Select::Released(lamp())]);
        // A pull that begins on nothing is nobody's, drawing nothing in a space, and a press of A across the lamp
        // selects it, as letting go of the pull does nothing to it.
        let away = Senses {
            ray: Ray::new(arrival::HOME, Vec3::Z),
            ..senses
        };
        frame(&mut step, &mut conversation, &away, Buttons::default());
        let mut pulling = away;
        pulling.trigger = Some(1.0);
        assert!(!frame(
            &mut step,
            &mut conversation,
            &pulling,
            Buttons::default()
        ));
        let mut across = senses;
        across.trigger = Some(1.0);
        frame(&mut step, &mut conversation, &across, Buttons::default());
        assert!(selects(&done).is_empty());
        frame(&mut step, &mut conversation, &across, a_down());
        assert_eq!(selects(&done), [Select::Pressed(lamp())]);
        assert!(!frame(&mut step, &mut conversation, &across, a_held()));
        across.trigger = Some(0.0);
        frame(&mut step, &mut conversation, &across, Buttons::default());
        assert_eq!(selects(&done), [Select::Released(lamp())]);
    }

    #[test]
    fn a_press_that_outlives_its_client_side_is_let_go_without_telling_the_next() {
        use crate::client_side::Select;
        let now = Instant::now();
        let (mut conversation, mut step, senses, done) = lit(Default::default());
        step.buttons(&a_down(), true, &mut conversation, &mut felt(), now);
        step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        assert_eq!(selects(&done), [Select::Pressed("lamp".into())]);
        // The space moves to an environment with no client side, and back, with A held throughout.
        for environment in ["minimal", "sky"] {
            conversation.apply(space(|state| state.environment = environment.into()));
            for _ in 0..400 {
                step.buttons(&a_held(), true, &mut conversation, &mut felt(), now);
                step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
            }
        }
        assert!(step.client.is_some());
        assert!(selects(&done).is_empty());
        // Let go, it tells the new client side nothing; the next press selects its lamp.
        step.buttons(
            &Buttons::default(),
            true,
            &mut conversation,
            &mut felt(),
            now,
        );
        step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        assert!(selects(&done).is_empty());
        step.buttons(&a_down(), true, &mut conversation, &mut felt(), now);
        step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        assert_eq!(selects(&done), [Select::Pressed("lamp".into())]);
    }

    #[test]
    fn a_press_waits_for_a_frame_that_is_drawn_and_is_told_once() {
        use crate::client_side::Select;
        let now = Instant::now();
        let lamp = || "lamp".to_owned();
        let (mut conversation, mut step, senses, done) = lit(Default::default());
        let read = |step: &mut Step, conversation: &mut Conversation, buttons: Buttons, focus| {
            step.buttons(&buttons, focus, conversation, &mut felt(), now)
        };
        let draw = |step: &mut Step, conversation: &mut Conversation| {
            step.update(&senses, conversation, &mut felt(), now, FRAME);
            selects(&done)
        };
        // The headset reads its buttons on frames it does not draw: a press, and A held after it, wait.
        read(&mut step, &mut conversation, a_down(), true);
        read(&mut step, &mut conversation, a_held(), true);
        read(&mut step, &mut conversation, a_held(), true);
        assert_eq!(
            draw(&mut step, &mut conversation),
            [Select::Pressed(lamp())]
        );
        assert!(draw(&mut step, &mut conversation).is_empty());
        read(&mut step, &mut conversation, Buttons::default(), true);
        assert_eq!(
            draw(&mut step, &mut conversation),
            [Select::Released(lamp())]
        );
        // Pressed and let go before the next frame drawn: pressed there, and let go on the one after.
        read(&mut step, &mut conversation, a_down(), true);
        read(&mut step, &mut conversation, Buttons::default(), true);
        assert_eq!(
            draw(&mut step, &mut conversation),
            [Select::Pressed(lamp())]
        );
        assert_eq!(
            draw(&mut step, &mut conversation),
            [Select::Released(lamp())]
        );
        assert!(draw(&mut step, &mut conversation).is_empty());
        // Pressed twice, it is still one select, as held and let go as it is.
        read(&mut step, &mut conversation, a_down(), true);
        read(&mut step, &mut conversation, a_down(), true);
        read(&mut step, &mut conversation, a_held(), true);
        assert_eq!(
            draw(&mut step, &mut conversation),
            [Select::Pressed(lamp())]
        );
        read(&mut step, &mut conversation, Buttons::default(), true);
        assert_eq!(
            draw(&mut step, &mut conversation),
            [Select::Released(lamp())]
        );
        // A press that waits is forgotten on losing focus, as the platform reports it or with `blur`.
        read(&mut step, &mut conversation, a_down(), true);
        read(&mut step, &mut conversation, a_held(), false);
        assert!(draw(&mut step, &mut conversation).is_empty());
        read(&mut step, &mut conversation, a_down(), true);
        step.blur(&mut conversation);
        assert!(draw(&mut step, &mut conversation).is_empty());
        // A held through a long skip, then drawn, is pressed once however many frames it was read on.
        for _ in 0..100 {
            read(&mut step, &mut conversation, a_held(), true);
        }
        assert!(draw(&mut step, &mut conversation).is_empty());
        read(&mut step, &mut conversation, Buttons::default(), true);
        assert!(draw(&mut step, &mut conversation).is_empty());
    }

    #[test]
    fn a_pressed_in_the_frame_the_hand_menu_appears_selects_nothing() {
        let now = Instant::now();
        let (mut conversation, mut step, mut senses, done) = lit(Default::default());
        // Y pins the menu, which shows as this frame is drawn, as A is pressed: the menu's, as the trigger's would be.
        step.buttons(
            &press(|buttons| {
                buttons.y = true;
                buttons.a = true;
                buttons.a_held = true;
            }),
            true,
            &mut conversation,
            &mut felt(),
            now,
        );
        senses.left_palm = Some(left_palm_up(0.0));
        step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        assert!(step.menu_was_shown);
        assert!(selects(&done).is_empty());
        // Held, then let go, it tells nothing, and the press is not kept for when the menu goes.
        step.buttons(&a_held(), true, &mut conversation, &mut felt(), now);
        step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        step.buttons(
            &Buttons::default(),
            true,
            &mut conversation,
            &mut felt(),
            now,
        );
        senses.left_palm = None;
        step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        assert!(selects(&done).is_empty());
    }

    #[test]
    fn a_pointing_at_something_of_tomori_s_in_front_of_a_selectable_object_is_tomori_s() {
        use tomori_protocol::{Card, CardKind, Checks, Panel, Place};
        let now = Instant::now();
        let ahead_of_home = Ray::new(arrival::HOME, Vec3::NEG_Z);
        // Each thing stands nearer than the lamp, 8 m along the ray, which the ray also meets, so that A goes to what is
        // nearer first, as the frame before found it. The checklist's item is real pointing, ticked by A, which the
        // conversation takes ahead of what else of Tomori's is pointed at.
        type Setup = fn(&mut Conversation, &mut Step, &mut Senses, Instant);
        let rows: [(&str, Setup); 5] = [
            ("a panel", |conversation, _, _, _| {
                fixtures::show_projection(
                    conversation,
                    Vec::new(),
                    vec![Panel::new(
                        "panel-1",
                        "Build status",
                        "Green.",
                        Place::Free {
                            pose: tomori_protocol::Pose::upright_facing(
                                (arrival::HOME + Vec3::new(0.0, 0.0, -2.0)).to_array(),
                                arrival::HOME.to_array(),
                            ),
                        },
                    )],
                );
            }),
            ("a checklist item", |conversation, step, senses, now| {
                let ahead = arrival::HOME + Vec3::new(0.0, 0.0, -2.0);
                fixtures::show_checklist(conversation, ahead, arrival::HOME);
                grow(step, conversation, senses, now);
                *senses = ray_on_item(step, conversation, ahead, "A  untick").expect("the item");
            }),
            ("a card", |conversation, _, _, _| {
                let at = arrival::HOME + Vec3::new(0.0, 0.0, -2.0);
                let (_, rotation, _) = crate::recording::facing(at, arrival::HOME, 1.0)
                    .to_scale_rotation_translation();
                fixtures::show_cards(
                    conversation,
                    vec![Card {
                        id: "github://o/r/issues/1".into(),
                        number: 1,
                        kind: CardKind::Issue,
                        title: "A card".into(),
                        author: "ada".into(),
                        labels: Vec::new(),
                        checks: Checks::None,
                        age: 0,
                        placement: Some(tomori_protocol::Pose {
                            position: at.to_array(),
                            orientation: rotation.to_array(),
                        }),
                    }],
                );
            }),
            ("a worker", |conversation, step, senses, now| {
                conversation.apply(space(|state| {
                    state.workers = vec![tomori_protocol::Worker {
                        id: "ses_a".into(),
                        parent: None,
                        title: "Review".into(),
                        state: tomori_protocol::WorkerState::Running,
                    }];
                }));
                let worker = step
                    .update(senses, conversation, &mut felt(), now, FRAME)
                    .stations[0]
                    .position;
                senses.ray = Ray::new(arrival::HOME, Vec3::from_array(worker) - arrival::HOME);
            }),
            ("a panel carried", |conversation, step, senses, now| {
                fixtures::show_projection(
                    conversation,
                    Vec::new(),
                    vec![Panel::new(
                        "panel-1",
                        "Build status",
                        "Green.",
                        Place::Free {
                            pose: tomori_protocol::Pose::upright_facing(
                                (arrival::HOME + Vec3::new(0.0, 0.0, -2.0)).to_array(),
                                arrival::HOME.to_array(),
                            ),
                        },
                    )],
                );
                senses.grip = Some(1.0);
                grow(step, conversation, senses, now);
                assert!(step.projector.held().is_some());
            }),
        ];
        for (label, setup) in rows {
            let (mut conversation, done) = lamp_at(Default::default(), true, 8.0);
            let mut step = Step::default();
            let mut senses = Senses {
                ray: ahead_of_home,
                beam: true,
                ..senses()
            };
            grow(&mut step, &mut conversation, &senses, now);
            setup(&mut conversation, &mut step, &mut senses, now);
            grow(&mut step, &mut conversation, &senses, now);
            assert_eq!(step.aim, precedence::Aim::Tomori, "{label}");
            conversation.take_actions();
            done.lock().unwrap().clear();
            step.buttons(&a_down(), true, &mut conversation, &mut felt(), now);
            step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
            step.buttons(
                &Buttons::default(),
                true,
                &mut conversation,
                &mut felt(),
                now,
            );
            step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
            // The environment is told nothing; A ticks the item pointed at, and does nothing else on the rest.
            assert!(selects(&done).is_empty(), "{label}");
            let tick = [Action::Send(
                tomori_protocol::ClientMessage::CheckPanelItem {
                    panel: "panel-1".into(),
                    line: 2,
                    checked: false,
                    revision: Some(4),
                },
            )];
            let actions = conversation.take_actions();
            if label == "a checklist item" {
                assert_eq!(actions, tick, "{label}");
            } else {
                assert!(actions.is_empty(), "{label}: {actions:?}");
            }
        }
    }

    /// The sticks a [`Lamp`] is told after `buttons`, `focused` or not, and a frame as `senses` find it.
    fn sticks(
        step: &mut Step,
        conversation: &mut Conversation,
        senses: &Senses,
        done: &Done,
        buttons: Buttons,
        focused: bool,
    ) -> (Vec2, Vec2) {
        step.buttons(&buttons, focused, conversation, &mut felt(), Instant::now());
        step.update(senses, conversation, &mut felt(), Instant::now(), FRAME);
        let last = done.lock().unwrap().last().cloned().unwrap();
        (last.2, last.3)
    }

    fn both(left: Vec2, right: Vec2) -> Buttons {
        press(|buttons| {
            buttons.left_stick = left;
            buttons.right_stick = right;
        })
    }

    #[test]
    fn the_left_stick_reaches_the_environment_only_where_it_travels_nowhere_from_the_start_of_a_push()
     {
        use crate::locomotion::{Locomotion, Surface};
        let push = both(Vec2::X, Vec2::ZERO);
        let rest = Buttons::default();
        // In an environment without travel, it is the environment's, but not without focus.
        let (mut conversation, mut step, senses, done) = lit(Locomotion::None);
        let told = |step: &mut Step, conversation: &mut Conversation, buttons, focused| {
            sticks(step, conversation, &senses, &done, buttons, focused).0
        };
        assert_eq!(told(&mut step, &mut conversation, push, true), Vec2::X);
        assert_eq!(told(&mut step, &mut conversation, push, false), Vec2::ZERO);
        // Where the environment lets the participant travel, it is travel's, until travel is turned off here, from the
        // next push: a push begun while it travelled stays travel's until it returns to the middle.
        let teleport =
            Locomotion::teleport(vec![Surface::rectangle(Vec3::ZERO, Vec2::splat(40.0))]);
        let (mut conversation, mut step, senses, done) = lit(teleport);
        let told = |step: &mut Step, conversation: &mut Conversation, buttons, focused| {
            sticks(step, conversation, &senses, &done, buttons, focused).0
        };
        assert_eq!(told(&mut step, &mut conversation, push, true), Vec2::ZERO);
        conversation.toggle_travel();
        assert_eq!(told(&mut step, &mut conversation, push, true), Vec2::ZERO);
        told(&mut step, &mut conversation, rest, true);
        assert_eq!(told(&mut step, &mut conversation, push, true), Vec2::X);
        // And the other way: a push begun as the environment's stays its own as travel is turned on again.
        conversation.toggle_travel();
        assert_eq!(told(&mut step, &mut conversation, push, true), Vec2::X);
        assert_eq!(step.travel_stick, Vec2::ZERO);
        told(&mut step, &mut conversation, rest, true);
        assert_eq!(told(&mut step, &mut conversation, push, true), Vec2::ZERO);
        // Losing focus forgets whose the push was.
        step.blur(&mut conversation);
        assert_eq!(step.left_stick, LeftStick::Free);
        assert_eq!(
            LeftStick::Other.update(Vec2::NAN, || LeftStick::Environment),
            LeftStick::Free
        );
    }

    #[test]
    fn the_right_stick_reaches_the_environment_only_where_nothing_else_takes_it() {
        let push = both(Vec2::ZERO, Vec2::NEG_Y);
        let rest = Buttons::default();
        // With nothing carried and the transcript closed, it is the environment's, whether or not its objects are
        // selectable, as the farmyard's chickens are not; but not without focus.
        for selectable in [true, false] {
            let (mut conversation, done) = lamp(Default::default(), selectable);
            let mut step = Step::default();
            let senses = Senses {
                ray: Ray::new(arrival::HOME, Vec3::NEG_Z),
                ..senses()
            };
            grow(&mut step, &mut conversation, &senses, Instant::now());
            let told = sticks(&mut step, &mut conversation, &senses, &done, push, true);
            assert_eq!(told.1, Vec2::NEG_Y);
            let told = sticks(&mut step, &mut conversation, &senses, &done, push, false);
            assert_eq!(told.1, Vec2::ZERO);
        }
        // While the right wrist's transcript shows, a push is its own.
        let (mut conversation, mut step, mut senses, done) = lit(Default::default());
        senses.right_palm = Some(palm_up(0.0));
        let now = Instant::now();
        wrist_after(&mut step, &mut conversation, &senses, now);
        assert!(step.wrist.state().active());
        let told = sticks(&mut step, &mut conversation, &senses, &done, push, true);
        assert_eq!(told.1, Vec2::ZERO);
        assert_eq!(step.right_stick, RightStick::Other);
        // While a panel is carried, a push is the panel's.
        let (mut conversation, mut step, mut senses, done) = lit(Default::default());
        let ahead = arrival::HOME + Vec3::new(0.0, 0.0, -0.6);
        fixtures::show_projection(
            &mut conversation,
            Vec::new(),
            vec![tomori_protocol::Panel::new(
                "panel-1",
                "Build status",
                "Green.",
                tomori_protocol::Place::Free {
                    pose: tomori_protocol::Pose::upright_facing(
                        ahead.to_array(),
                        arrival::HOME.to_array(),
                    ),
                },
            )],
        );
        senses.grip = Some(1.0);
        grow(&mut step, &mut conversation, &senses, now);
        assert!(step.projector.held().is_some());
        sticks(&mut step, &mut conversation, &senses, &done, rest, true);
        let told = sticks(&mut step, &mut conversation, &senses, &done, push, true);
        assert_eq!(told.1, Vec2::ZERO);
        assert_eq!(step.right_stick, RightStick::Carried);
        // A push is the environment's only if nothing else takes it as it leaves the middle, until it returns there.
        assert_eq!(
            RightStick::Environment.update(Vec2::Y, || RightStick::Carried),
            RightStick::Environment
        );
    }

    /// A code panel of `lines` lines floating 0.6 m ahead, nearer than the lamp, so that the ray points at it.
    fn code_ahead(conversation: &mut Conversation, lines: u32) {
        use tomori_protocol::{Code, Panel, Place, Pose};
        let ahead = arrival::HOME + Vec3::new(0.0, 0.0, -0.6);
        let body: Vec<String> = (1..=lines)
            .map(|line| format!("let x{line} = {line};"))
            .collect();
        fixtures::show_projection(
            conversation,
            Vec::new(),
            vec![
                Panel::new(
                    "panel-1",
                    "rust",
                    body.join("\n"),
                    Place::Free {
                        pose: Pose::upright_facing(ahead.to_array(), arrival::HOME.to_array()),
                    },
                )
                .with_code(Code::new("rust", lines)),
            ],
        );
    }

    #[test]
    fn pointing_at_a_panel_the_sticks_are_tomori_s_and_never_the_environment_s() {
        use crate::locomotion::Locomotion;
        let push = both(Vec2::X, Vec2::NEG_Y);
        let rest = Buttons::default();
        let now = Instant::now();
        // In an environment without travel, whose client side would take both sticks, a panel in front of its lamp
        // takes them: one with more than it shows scrolls with the right stick, one that shows everything takes it to
        // do nothing, and neither gives the left stick to the environment.
        for (lines, scrolls) in [(200, true), (1, false)] {
            let (mut conversation, mut step, senses, done) = lit(Locomotion::None);
            code_ahead(&mut conversation, lines);
            grow(&mut step, &mut conversation, &senses, now);
            assert_eq!(step.aim, precedence::Aim::Tomori);
            assert_eq!(step.projector.scrollable("panel-1"), scrolls);
            let told = sticks(&mut step, &mut conversation, &senses, &done, push, true);
            assert_eq!(told, (Vec2::ZERO, Vec2::ZERO), "{lines}");
            assert_eq!(step.left_stick, LeftStick::Other);
            if scrolls {
                assert_eq!(step.right_stick, RightStick::Scrolled("panel-1".into()));
                step.update(&senses, &mut conversation, &mut felt(), now, 0.5);
                assert!(step.projector.scroll("panel-1") > 0);
            } else {
                assert_eq!(step.right_stick, RightStick::Other);
            }
            // A push begun on the panel stays Tomori's as the ray moves off it, until it returns to the middle.
            let away = Senses {
                ray: Ray::new(arrival::HOME, Vec3::Z),
                ..senses
            };
            let told = sticks(&mut step, &mut conversation, &away, &done, push, true);
            assert_eq!(told, (Vec2::ZERO, Vec2::ZERO), "{lines}");
            // Back to the middle with the ray on nothing of Tomori's, nor of the environment's, the next push is the
            // environment's.
            sticks(&mut step, &mut conversation, &away, &done, rest, true);
            assert_eq!(step.aim, precedence::Aim::Nothing);
            let told = sticks(&mut step, &mut conversation, &away, &done, push, true);
            assert_eq!(told, (Vec2::X, Vec2::NEG_Y), "{lines}");
            // Carried, the panel takes the right stick before anything else, and the left stick is not the
            // environment's.
            sticks(&mut step, &mut conversation, &senses, &done, rest, true);
            let gripped = Senses {
                grip: Some(1.0),
                ..senses
            };
            step.update(&gripped, &mut conversation, &mut felt(), now, FRAME);
            assert_eq!(step.projector.held(), Some("panel-1"));
            let told = sticks(&mut step, &mut conversation, &gripped, &done, push, true);
            assert_eq!(told, (Vec2::ZERO, Vec2::ZERO), "{lines}");
            assert_eq!(step.right_stick, RightStick::Carried);
            assert_eq!(step.left_stick, LeftStick::Other);
        }
    }

    #[test]
    fn a_trigger_pull_on_a_panel_in_front_of_a_selectable_object_is_tomori_s() {
        let (mut conversation, mut step, mut senses, done) = lit(Default::default());
        let now = Instant::now();
        code_ahead(&mut conversation, 1);
        grow(&mut step, &mut conversation, &senses, now);
        assert_eq!(step.aim, precedence::Aim::Tomori);
        done.lock().unwrap().clear();
        // The pull attracts the swarm, as on any panel, and selects nothing behind it.
        let mut attracting = Vec::new();
        for trigger in [0.3, 1.0, 1.0, 0.0] {
            senses.trigger = Some(trigger);
            attracting.push(
                step.update(&senses, &mut conversation, &mut felt(), now, FRAME)
                    .attracting,
            );
        }
        assert_eq!(attracting, [false; 4]);
        assert!(selects(&done).is_empty());
        // Nor does a pull begun on the panel select the lamp once the panel has gone from in front of it.
        senses.trigger = Some(1.0);
        step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        fixtures::show_projection(&mut conversation, Vec::new(), Vec::new());
        for _ in 0..4 {
            step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        }
        assert_eq!(step.aim, precedence::Aim::Environment);
        assert!(selects(&done).is_empty());
        // Let go, the next pull on the lamp is the environment's.
        senses.trigger = Some(0.0);
        step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        senses.trigger = Some(1.0);
        step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        assert_eq!(
            selects(&done),
            [crate::client_side::Select::Pressed("lamp".into())]
        );
    }

    #[test]
    fn without_a_client_side_the_right_stick_pages_no_result_card_in_a_space() {
        // In a space whose environment has no client side, a push is the cards', as ever, which never page there.
        let mut conversation = joined();
        let mut step = Step::default();
        let senses = senses();
        let now = Instant::now();
        grow(&mut step, &mut conversation, &senses, now);
        assert!(step.client.is_none());
        for direction in [Vec2::X, Vec2::NEG_X, Vec2::Y, Vec2::NEG_Y] {
            step.buttons(
                &both(Vec2::ZERO, direction),
                true,
                &mut conversation,
                &mut felt(),
                now,
            );
            assert_eq!(step.right_stick, RightStick::Other);
            let frame = step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
            assert_eq!((frame.card, frame.content), (0, 0));
            step.buttons(
                &Buttons::default(),
                true,
                &mut conversation,
                &mut felt(),
                now,
            );
        }
    }

    #[test]
    fn travel_waits_for_the_space_s_scenery_to_grow_in() {
        use crate::locomotion::{Locomotion, Surface};
        let mut conversation = lobby(ConnectionRole::Owner);
        conversation.set_locomotion(vec![(
            crate::test_support::SKY,
            Locomotion::teleport(vec![Surface::rectangle(Vec3::ZERO, Vec2::splat(40.0))]),
        )]);
        let mut step = Step::default();
        let now = Instant::now();
        let senses = Senses {
            ray: Ray::new(arrival::HOME, Vec3::new(0.0, -1.0, -1.0)),
            ..senses()
        };
        step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        conversation.apply(space(|_| {}));
        step.buttons(
            &press(|buttons| buttons.left_stick = Vec2::Y),
            true,
            &mut conversation,
            &mut felt(),
            now,
        );
        let growing = step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        assert_ne!(
            growing.scenery,
            arrival_motion::Scenery {
                environment: crate::test_support::SKY,
                reveal: None
            }
        );
        assert_eq!(growing.landing, None);
        let mut landing = None;
        for _ in 0..400 {
            landing = step
                .update(&senses, &mut conversation, &mut felt(), now, FRAME)
                .landing;
        }
        // A push held from before it grew in aims nothing: the stick comes back to the middle first.
        assert_eq!(landing, None);
        for stick in [Vec2::ZERO, Vec2::Y] {
            step.buttons(
                &press(|buttons| buttons.left_stick = stick),
                true,
                &mut conversation,
                &mut felt(),
                now,
            );
            landing = step
                .update(&senses, &mut conversation, &mut felt(), now, FRAME)
                .landing;
        }
        assert!(landing.is_some(), "once it has grown in");
    }

    #[test]
    fn pointing_at_a_more_sign_as_words_begin_attends_to_the_workers_it_stands_for() {
        use tomori_protocol::Resource;
        use tomori_protocol::cluster::{Anchor as Named, HOME, Layout};
        let mut conversation = lobby(ConnectionRole::Owner);
        // A home of little room, and more workers than it holds even compact, seated by the server.
        let home = tomori_protocol::Pose::at(crate::swarm::CENTER.to_array())
            .facing_towards([0.0, 1.6, 0.0]);
        let count = 16;
        conversation.apply(space(|state| {
            state.workers = (0..count)
                .map(|index| tomori_protocol::Worker {
                    id: format!("ses_{index:02}"),
                    parent: None,
                    title: format!("Look at {index}"),
                    state: tomori_protocol::WorkerState::Running,
                })
                .collect();
            state.anchors = tomori_protocol::Anchors::new()
                .with_workers(home)
                .with_cluster(Named::new(HOME, home).with_extent(4.2, 1.6, 2.0));
            state.clusters = vec![(0..count).fold(Layout::new(HOME, count), |layout, slot| {
                layout.with_seat(format!("ses_{slot:02}"), slot)
            })];
        }));
        let mut step = Step::default();
        let mut feedback = felt();
        let now = Instant::now();
        let mut senses = Senses {
            beam: true,
            ..senses()
        };
        let mut sign = None;
        for _ in 0..400 {
            let output = step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
            sign = output
                .stations
                .iter()
                .find(|station| station.more > 0)
                .cloned();
        }
        let sign = Vec3::from_array(sign.expect("a sign for those it does not hold").position);
        // From beneath it, where no other worker stands.
        senses.ray = Ray::new(sign - Vec3::Y * 0.8, Vec3::Y);
        step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
        conversation.take_actions();
        conversation.talk(true);
        conversation.talk(false);
        let actions = conversation.take_actions();
        let Some(Action::BeginUtterance(utterance)) = actions.first() else {
            unreachable!("recording begins, not {actions:?}")
        };
        assert_eq!(utterance.attention, [Resource::gathered(HOME)]);
    }

    #[test]
    fn aiming_a_teleport_takes_the_ray_from_workers_and_the_hand_menu_or_focus_cancels_it() {
        use crate::locomotion::{Locomotion, Surface};
        use tomori_protocol::Resource;
        let mut conversation = lobby(ConnectionRole::Owner);
        conversation.set_locomotion(vec![(
            crate::test_support::SKY,
            Locomotion::teleport(vec![Surface::rectangle(Vec3::ZERO, Vec2::splat(40.0))]),
        )]);
        conversation.apply(space(|state| {
            state.workers = vec![tomori_protocol::Worker {
                id: "ses_a".into(),
                parent: None,
                title: "Review".into(),
                state: tomori_protocol::WorkerState::Running,
            }];
        }));
        let mut step = Step::default();
        let mut feedback = felt();
        let now = Instant::now();
        let mut senses = Senses {
            beam: true,
            ..senses()
        };
        // Travel waits for the space's scenery to grow in.
        for _ in 0..400 {
            step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
        }
        let worker = Vec3::from_array(
            step.update(&senses, &mut conversation, &mut feedback, now, FRAME)
                .stations[0]
                .position,
        );
        // At the top of the sphere that points at it, nearly level.
        senses.ray = Ray::new(arrival::HOME, worker + Vec3::Y * 0.28 - arrival::HOME);
        let attending = |conversation: &mut Conversation| {
            conversation.talk(true);
            conversation.talk(false);
            let actions = conversation.take_actions();
            let Some(Action::BeginUtterance(utterance)) = actions.first() else {
                unreachable!("recording begins, not {actions:?}")
            };
            utterance.attention.clone()
        };
        let stick = |step: &mut Step, conversation: &mut Conversation, stick: Vec2| {
            step.buttons(
                &press(|buttons| buttons.left_stick = stick),
                true,
                conversation,
                &mut felt(),
                now,
            );
        };
        conversation.take_actions();
        step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
        assert_eq!(attending(&mut conversation), [Resource::worker("ses_a")]);
        // Aiming, the worker is no longer pointed at. The ray at it meets the floor only beyond travel's reach, so it has nowhere to land, and is drawn plain and short of it.
        stick(&mut step, &mut conversation, Vec2::Y);
        let output = step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
        let beam = output
            .arrival
            .as_ref()
            .and_then(|arrival| arrival.ray)
            .unwrap();
        assert_eq!(output.landing, None);
        assert!(!beam.hit);
        let short = senses.ray.unwrap().at(crate::pointing::SPACE_RAY_LENGTH);
        assert!(beam.end.distance(short) < 1e-4);
        assert!(attending(&mut conversation).is_empty());
        // Down at the floor, it lands; with the hand menu showing, the aim is dropped and releasing does nothing.
        senses.ray = Ray::new(arrival::HOME, Vec3::new(0.0, -1.0, -1.0));
        assert!(
            step.update(&senses, &mut conversation, &mut feedback, now, FRAME)
                .landing
                .is_some()
        );
        senses.left_palm = Some(left_palm_up(0.0));
        step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
        let shown = step.update(&senses, &mut conversation, &mut feedback, now + HOLD, FRAME);
        assert_eq!(shown.landing, None);
        senses.left_palm = None;
        stick(&mut step, &mut conversation, Vec2::ZERO);
        step.update(&senses, &mut conversation, &mut feedback, now + HOLD, FRAME);
        assert_eq!(step.rig(&conversation), Rig::default());
        // Losing focus drops it too.
        stick(&mut step, &mut conversation, Vec2::Y);
        step.update(&senses, &mut conversation, &mut feedback, now + HOLD, FRAME);
        step.blur(&mut conversation);
        step.update(&senses, &mut conversation, &mut feedback, now + HOLD, FRAME);
        assert_eq!(step.rig(&conversation), Rig::default());
        // Unfocused buttons hold the stick in the middle.
        step.buttons(
            &press(|buttons| buttons.left_stick = Vec2::Y),
            false,
            &mut conversation,
            &mut felt(),
            now,
        );
        assert_eq!(step.travel_stick, Vec2::ZERO);
    }

    /// A main floor and racks 4.5 m below it, with a pad 2 m ahead of home and another on the racks.
    fn storeys() -> crate::locomotion::Locomotion {
        use crate::locomotion::{Locomotion, Pad, Surface};
        Locomotion::teleport(vec![
            Surface::rectangle(Vec3::ZERO, Vec2::splat(40.0)),
            Surface::rectangle(Vec3::new(0.0, -4.5, 0.0), Vec2::splat(40.0)),
        ])
        .with_pads([[
            Pad::at(Vec3::new(0.0, 0.0, -2.0)).named("Main floor"),
            Pad::at(Vec3::new(3.0, -4.5, -2.0)).named("Racks"),
        ]])
    }

    /// The senses of a head at home, with the trigger at `trigger` and the ray from the head towards `towards`, in the
    /// room, both placed with the rig the step settles for `conversation`, as the platform places them.
    fn on_pads(
        step: &mut Step,
        conversation: &Conversation,
        towards: Vec3,
        trigger: f32,
    ) -> Senses {
        let rig = step.rig(conversation).matrix();
        let head = rig * Mat4::from_translation(arrival::HOME);
        Senses {
            beam: true,
            head: Some(head),
            head_pose: Pose::from_transform(head),
            viewer: Some(head.w_axis.truncate()),
            ray: Ray::new(
                rig.transform_point3(arrival::HOME),
                rig.transform_vector3(towards - arrival::HOME),
            ),
            trigger: Some(trigger),
            ..senses()
        }
    }

    #[test]
    fn a_pad_pointed_at_is_highlighted_and_labelled_and_the_trigger_or_a_takes_the_participant_to_its_partner()
     {
        let mut conversation = lobby(ConnectionRole::Owner);
        conversation.set_locomotion(vec![(crate::test_support::SKY, storeys())]);
        conversation.apply(space(|_| {}));
        let mut step = Step::default();
        let mut feedback = felt();
        let now = Instant::now();
        let pad = Vec3::new(0.0, 0.2, -2.0);
        // Before the scenery has grown in, no pad is drawn or pointed at.
        let senses = on_pads(&mut step, &conversation, pad, 0.0);
        let growing = step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
        assert!(growing.pads.is_empty());
        for _ in 0..400 {
            step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
        }
        // Pointed at, it is highlighted, its label says where it goes, the ray ends on its column, and it ticks.
        let senses = on_pads(&mut step, &conversation, Vec3::new(0.0, 1.6, -5.0), 0.0);
        let frame = step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
        assert_eq!(frame.pads.len(), 2);
        assert!(frame.pads.iter().all(|pad| !pad.highlighted));
        let later = now + Duration::from_secs(5);
        felt_on(&mut feedback, Side::Right, later);
        let senses = on_pads(&mut step, &conversation, pad, 0.0);
        let frame = step.update(&senses, &mut conversation, &mut feedback, later, FRAME);
        assert!(frame.pads[0].highlighted && !frame.pads[1].highlighted);
        assert_eq!(frame.pads[0].up, Some(false));
        let labels: Vec<String> = frame
            .projection
            .iter()
            .map(|sheet| sheet.canvas.as_deref().unwrap().plain())
            .collect();
        assert!(
            labels.iter().any(|label| label.starts_with("Racks ↓")),
            "{labels:?}"
        );
        let beam = frame
            .arrival
            .as_ref()
            .and_then(|arrival| arrival.ray)
            .unwrap();
        assert!(beam.hit);
        assert!((beam.end.z + 1.5).abs() < 1e-3, "{}", beam.end);
        assert!(
            !felt_on(&mut feedback, Side::Right, later).is_empty(),
            "it ticks"
        );
        earcons(&mut feedback);
        // The trigger, pulled on it, takes the participant down to the racks, blinking, without attracting the swarm.
        let senses = on_pads(&mut step, &conversation, pad, 1.0);
        let frame = step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
        assert!(!frame.attracting);
        assert_eq!(frame.blink, 1.0);
        assert_eq!(earcons(&mut feedback), [Earcon::Choose]);
        let rig = step.rig(&conversation);
        assert_eq!(rig.floor(), -4.5);
        let head = rig.matrix().transform_point3(arrival::HOME);
        assert!(
            head.distance(Vec3::new(3.0, 1.6 - 4.5, -2.0)) < 1e-4,
            "{head}"
        );
        // Held there, the pull neither attracts nor takes them anywhere else; they stand on the racks' pad, disarmed.
        for _ in 0..60 {
            let senses = on_pads(&mut step, &conversation, Vec3::new(0.0, 1.6, -5.0), 1.0);
            let frame = step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
            assert!(!frame.attracting && frame.blink < 1.0);
        }
        assert_eq!(step.rig(&conversation), rig);
        // Pointing down at the pad stood on, A takes them back up, clicking once.
        let feet = Vec3::new(0.0, 0.0, -0.05);
        let senses = on_pads(&mut step, &conversation, feet, 0.0);
        let frame = step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
        assert!(frame.pads[1].highlighted, "{:?}", frame.pads);
        assert!(frame.projection.iter().any(|sheet| {
            sheet
                .canvas
                .as_deref()
                .unwrap()
                .plain()
                .starts_with("Main floor ↑")
        }));
        earcons(&mut feedback);
        step.buttons(
            &press(|buttons| buttons.a = true),
            true,
            &mut conversation,
            &mut feedback,
            now,
        );
        let frame = step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
        assert_eq!(frame.blink, 1.0);
        assert_eq!(earcons(&mut feedback), [Earcon::Choose]);
        let head = step
            .rig(&conversation)
            .matrix()
            .transform_point3(arrival::HOME);
        assert!(head.distance(Vec3::new(0.0, 1.6, -2.0)) < 1e-4, "{head}");
        // A pointing at nothing then chooses no pad.
        let senses = on_pads(&mut step, &conversation, Vec3::new(5.0, 1.6, -2.0), 0.0);
        step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
        let rig = step.rig(&conversation);
        step.buttons(
            &press(|buttons| buttons.a = true),
            true,
            &mut conversation,
            &mut feedback,
            now,
        );
        step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
        assert_eq!(step.rig(&conversation), rig);
    }

    #[test]
    fn a_pad_gives_way_to_anything_nearer_and_to_aiming_and_is_not_there_without_travel() {
        use tomori_protocol::{Panel, Place};
        let mut conversation = lobby(ConnectionRole::Owner);
        conversation.set_locomotion(vec![(crate::test_support::SKY, storeys())]);
        // A panel floating between home and the pad.
        let between = arrival::HOME + Vec3::new(0.0, -0.6, -1.0);
        fixtures::show_projection(
            &mut conversation,
            Vec::new(),
            vec![Panel::new(
                "panel-1",
                "Build status",
                "Green.",
                Place::Free {
                    pose: tomori_protocol::Pose::upright_facing(
                        between.to_array(),
                        arrival::HOME.to_array(),
                    ),
                },
            )],
        );
        let mut step = Step::default();
        let mut feedback = felt();
        let now = Instant::now();
        // Through the panel's middle to the pad's column behind it.
        let pad = Vec3::new(0.0, 0.4, -2.0);
        let senses = on_pads(&mut step, &conversation, pad, 0.0);
        for _ in 0..400 {
            step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
        }
        // Behind the panel, the pad is not pointed at, and the trigger is the panel's, attracting the swarm as ever.
        let frame = step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
        assert!(frame.pads.iter().all(|pad| !pad.highlighted));
        assert_eq!(step.aim, precedence::Aim::Tomori);
        let senses = Senses {
            attractor: Some(pad),
            ..on_pads(&mut step, &conversation, pad, 1.0)
        };
        let frame = step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
        assert!(!frame.attracting);
        assert_eq!(step.rig(&conversation), Rig::default());
        // Aiming a teleport, the ray serves only it.
        // From the side, so that the panel is not in the way.
        let from = Vec3::new(2.0, 1.3, 0.0);
        let senses = Senses {
            ray: Ray::new(from, Vec3::new(0.0, 0.3, -2.0) - from),
            ..on_pads(&mut step, &conversation, pad, 0.0)
        };
        step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
        assert_eq!(step.aim, precedence::Aim::Pad);
        step.buttons(
            &press(|buttons| buttons.left_stick = Vec2::Y),
            true,
            &mut conversation,
            &mut feedback,
            now,
        );
        let frame = step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
        assert!(frame.landing.is_some());
        assert!(frame.pads.iter().all(|pad| !pad.highlighted));
        step.cancel_travel();
        // With travel turned off here, no pad is drawn or pointed at.
        conversation.toggle_travel();
        let frame = step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
        assert!(frame.pads.is_empty());
        assert_ne!(step.aim, precedence::Aim::Pad);
    }

    /// A profile with a pad on a disc 1 m beyond where `ray` meets something `nearer`, with its column 0.3 m up the ray
    /// there, so that the ray would point at it but for what is nearer, on a disc too small for the ray to cross.
    fn pad_behind(ray: &Ray, nearer: f32) -> crate::locomotion::Locomotion {
        use crate::locomotion::{Locomotion, Pad, Surface};
        let at = ray.at(nearer + 1.0);
        let base = Vec3::new(at.x, at.y - 0.3, at.z);
        // Its partner stands a little further along the ray, so that it is no nearer.
        let along = Vec3::new(ray.direction.x, 0.0, ray.direction.z).normalize_or_zero();
        Locomotion::teleport(vec![Surface::disc(base, 0.9)])
            .with_pads([[Pad::at(base), Pad::at(base + along * 0.4)]])
    }

    /// With `ray` meeting something `nearer`, which makes the aim `aim`, the pad behind it is not pointed at, though it
    /// would be alone.
    fn behind_it_no_pad(
        conversation: &mut Conversation,
        step: &mut Step,
        ray: Ray,
        nearer: f32,
        aim: precedence::Aim,
    ) {
        let profile = pad_behind(&ray, nearer);
        let (_, distance, _) = crate::pads::pick(&profile, &ray).expect("alone, it is pointed at");
        assert!(distance > nearer, "{distance} {nearer}");
        conversation.set_locomotion(vec![(crate::test_support::SKY, profile)]);
        let senses = Senses {
            beam: true,
            ray: Some(ray),
            ..senses()
        };
        let frame = step.update(&senses, conversation, &mut felt(), Instant::now(), FRAME);
        assert!(
            frame.pads.iter().all(|pad| !pad.highlighted),
            "{aim:?} {nearer}"
        );
        assert_eq!(step.aim, aim);
    }

    #[test]
    fn a_pad_is_not_pointed_at_behind_a_card_a_worker_an_environment_s_object_or_a_wall() {
        use precedence::Aim;
        use tomori_protocol::{Pose, Wall, WallKind};
        let now = Instant::now();
        let ahead = senses();
        let grown = |conversation: &mut Conversation| {
            let mut step = Step::default();
            grow(&mut step, conversation, &ahead, now);
            step
        };
        // A card, wherever it stands, which a ray through its middle meets.
        let mut conversation = lobby(ConnectionRole::Owner);
        fixtures::show_cards(
            &mut conversation,
            vec![tomori_protocol::Card {
                id: "github://o/r/issues/1".into(),
                number: 1,
                kind: tomori_protocol::CardKind::Issue,
                title: "A card".into(),
                author: "ada".into(),
                labels: Vec::new(),
                checks: tomori_protocol::Checks::Passing,
                age: 0,
                placement: None,
            }],
        );
        let mut step = grown(&mut conversation);
        let frame = step.update(&ahead, &mut conversation, &mut felt(), now, FRAME);
        let card = frame.cards[0].model.expect("the card").w_axis.truncate();
        let ray = Ray::new(arrival::HOME, card - arrival::HOME).unwrap();
        behind_it_no_pad(
            &mut conversation,
            &mut step,
            ray,
            card.distance(arrival::HOME),
            Aim::Tomori,
        );
        // A worker, met at the top of the sphere that points at it.
        let mut conversation = lobby(ConnectionRole::Owner);
        conversation.apply(space(|state| {
            state.workers = vec![tomori_protocol::Worker {
                id: "ses_a".into(),
                parent: None,
                title: "Review".into(),
                state: tomori_protocol::WorkerState::Running,
            }];
        }));
        let mut step = grown(&mut conversation);
        let worker = Vec3::from_array(
            step.update(&ahead, &mut conversation, &mut felt(), now, FRAME)
                .stations[0]
                .position,
        ) + Vec3::Y * 0.28;
        let ray = Ray::new(arrival::HOME, worker - arrival::HOME).unwrap();
        behind_it_no_pad(
            &mut conversation,
            &mut step,
            ray,
            worker.distance(arrival::HOME) - 0.3,
            Aim::Tomori,
        );
        // An environment's wall's board, where the participant may take hold of walls.
        let wall = Wall::new(
            "environment-0",
            WallKind::Environment,
            Pose::upright_facing([1.2, 1.5, -2.5], arrival::HOME.to_array()),
        );
        let board = Vec3::new(1.2, 1.5 + 0.42, -2.5);
        let mut conversation = lobby(ConnectionRole::Owner);
        fixtures::show_editable_walls(&mut conversation, vec![wall], Vec::new());
        let mut step = grown(&mut conversation);
        let ray = Ray::new(arrival::HOME, board - arrival::HOME).unwrap();
        behind_it_no_pad(
            &mut conversation,
            &mut step,
            ray,
            board.distance(arrival::HOME),
            Aim::Tomori,
        );
        // An environment's object, selectable or not, a metre away.
        for selectable in [true, false] {
            let (mut conversation, _) = lamp(Default::default(), selectable);
            let mut step = grown(&mut conversation);
            let ray = Ray::new(arrival::HOME, Vec3::new(0.0, -0.2, -1.0)).unwrap();
            behind_it_no_pad(&mut conversation, &mut step, ray, 1.0, Aim::Environment);
        }
    }

    /// A profile with a pad 2 m behind home, whose partner is on the racks below.
    fn pad_behind_home() -> crate::locomotion::Locomotion {
        use crate::locomotion::{Locomotion, Pad, Surface};
        Locomotion::teleport(vec![
            Surface::rectangle(Vec3::ZERO, Vec2::splat(40.0)),
            Surface::rectangle(Vec3::new(0.0, -4.5, 0.0), Vec2::splat(40.0)),
        ])
        .with_pads([[
            Pad::at(Vec3::new(0.0, 0.0, 2.0)),
            Pad::at(Vec3::new(3.0, -4.5, 2.0)),
        ]])
    }

    #[test]
    fn a_pull_begun_elsewhere_that_slides_onto_a_pad_chooses_nothing_and_a_still_goes_while_the_environment_holds_a_select()
     {
        use crate::client_side::Select;
        let now = Instant::now();
        let at_pad = Ray::new(arrival::HOME, Vec3::new(0.0, 0.3, 2.0) - arrival::HOME);
        let at_nothing = Ray::new(arrival::HOME, Vec3::X);
        let aimed = |ray: Option<Ray>, trigger: f32| Senses {
            ray,
            trigger: Some(trigger),
            ..senses()
        };
        // A pull that begins on nothing attracts the swarm, and sliding onto the pad then, even pulled fully, chooses it
        // no more than letting go does.
        let (mut conversation, mut step, _, _) = lit(pad_behind_home());
        step.update(
            &aimed(at_nothing, 0.0),
            &mut conversation,
            &mut felt(),
            now,
            FRAME,
        );
        let frame = step.update(
            &aimed(at_nothing, 0.3),
            &mut conversation,
            &mut felt(),
            now,
            FRAME,
        );
        assert!(!frame.attracting);
        for trigger in [0.3, 1.0, 1.0, 0.0] {
            let frame = step.update(
                &aimed(at_pad, trigger),
                &mut conversation,
                &mut felt(),
                now,
                FRAME,
            );
            assert_eq!(
                frame.pads.iter().filter(|pad| pad.highlighted).count(),
                usize::from(trigger > 0.0 || frame.pads[0].highlighted)
            );
        }
        assert_eq!(step.rig(&conversation), Rig::default());
        // The environment's select, held from the lamp, and A on a pad slid onto: A goes, the select is pressed and let
        // go once, and the pull chooses no pad.
        let (mut conversation, mut step, _, done) = lit(pad_behind_home());
        let ahead = Ray::new(arrival::HOME, Vec3::NEG_Z);
        step.update(
            &aimed(ahead, 0.7),
            &mut conversation,
            &mut felt(),
            now,
            FRAME,
        );
        let frame = step.update(
            &aimed(at_pad, 0.7),
            &mut conversation,
            &mut felt(),
            now,
            FRAME,
        );
        assert!(frame.pads[0].highlighted && !frame.attracting);
        assert_eq!(step.rig(&conversation), Rig::default());
        step.buttons(
            &press(|buttons| buttons.a = true),
            true,
            &mut conversation,
            &mut felt(),
            now,
        );
        step.update(
            &aimed(at_pad, 0.7),
            &mut conversation,
            &mut felt(),
            now,
            FRAME,
        );
        assert_eq!(step.rig(&conversation).floor(), -4.5);
        step.update(
            &aimed(at_pad, 0.0),
            &mut conversation,
            &mut felt(),
            now,
            FRAME,
        );
        assert_eq!(
            selects(&done),
            [
                Select::Pressed("lamp".into()),
                Select::Released("lamp".into())
            ]
        );
    }

    #[test]
    fn a_pull_on_a_row_or_a_pad_while_a_holds_the_environment_s_select_is_nobody_s() {
        use crate::client_side::Select;
        let now = Instant::now();
        // A row: behind home, where the lamp, which picks only rays ahead, is not.
        let (mut conversation, mut step, ahead, done) = lit(Default::default());
        let behind = arrival::HOME + Vec3::new(0.0, 0.0, 2.0);
        show_queue(&mut conversation, behind, arrival::HOME);
        let row = queue_rows(&mut step, &mut conversation, behind)[1];
        let pulled = Senses {
            trigger: Some(1.0),
            ..row
        };
        step.update(&ahead, &mut conversation, &mut felt(), now, FRAME);
        step.buttons(&a_down(), true, &mut conversation, &mut felt(), now);
        step.update(&ahead, &mut conversation, &mut felt(), now, FRAME);
        assert_eq!(selects(&done), [Select::Pressed("lamp".into())]);
        conversation.take_actions();
        step.update(&row, &mut conversation, &mut felt(), now, FRAME);
        for _ in 0..2 {
            step.buttons(&a_held(), true, &mut conversation, &mut felt(), now);
            step.update(&pulled, &mut conversation, &mut felt(), now, FRAME);
        }
        // A let go while the pull lasts, the pull is still nobody's.
        step.buttons(
            &Buttons::default(),
            true,
            &mut conversation,
            &mut felt(),
            now,
        );
        step.update(&pulled, &mut conversation, &mut felt(), now, FRAME);
        assert_eq!(selects(&done), [Select::Released("lamp".into())]);
        assert!(conversation.take_actions().is_empty());
        // Let go, and pulled again, it chooses the row.
        step.update(&row, &mut conversation, &mut felt(), now, FRAME);
        step.update(&pulled, &mut conversation, &mut felt(), now, FRAME);
        assert_eq!(
            conversation.take_actions(),
            [Action::Send(
                tomori_protocol::ClientMessage::ChoosePanelRow {
                    panel: "panel-2".into(),
                    row: "pod112".into(),
                    revision: Some(6),
                }
            )]
        );
        // A pad: the same.
        let (mut conversation, mut step, ahead, done) = lit(pad_behind_home());
        let at_pad = Senses {
            ray: Ray::new(arrival::HOME, Vec3::new(0.0, 0.3, 2.0) - arrival::HOME),
            ..ahead
        };
        let pulled = Senses {
            trigger: Some(1.0),
            ..at_pad
        };
        step.buttons(&a_down(), true, &mut conversation, &mut felt(), now);
        step.update(&ahead, &mut conversation, &mut felt(), now, FRAME);
        assert_eq!(selects(&done), [Select::Pressed("lamp".into())]);
        step.update(&at_pad, &mut conversation, &mut felt(), now, FRAME);
        step.buttons(&a_held(), true, &mut conversation, &mut felt(), now);
        step.update(&pulled, &mut conversation, &mut felt(), now, FRAME);
        step.update(&pulled, &mut conversation, &mut felt(), now, FRAME);
        assert_eq!(step.rig(&conversation), Rig::default());
        step.buttons(
            &Buttons::default(),
            true,
            &mut conversation,
            &mut felt(),
            now,
        );
        step.update(&at_pad, &mut conversation, &mut felt(), now, FRAME);
        step.update(&pulled, &mut conversation, &mut felt(), now, FRAME);
        assert_eq!(step.rig(&conversation).floor(), -4.5);
    }

    #[test]
    fn a_panel_behind_a_pad_is_not_pulled_off_by_the_grip() {
        use crate::locomotion::{Locomotion, Pad, Surface};
        use tomori_protocol::{Panel, Place};
        let now = Instant::now();
        // A low ray along the floor, to the right of the Tomori wall, which meets the pad's column 2 m ahead and a panel
        // 1 m behind it.
        let behind = Vec3::new(2.0, 0.4, -3.0);
        let ray = Ray::new(Vec3::new(2.0, 0.5, 0.0), Vec3::new(0.0, -0.05, -1.5));
        let held = |grip: f32| Senses {
            ray,
            grip: Some(grip),
            ..senses()
        };
        for pad in [false, true] {
            let mut conversation = lobby(ConnectionRole::Owner);
            fixtures::show_projection(
                &mut conversation,
                Vec::new(),
                vec![Panel::new(
                    "panel-1",
                    "Behind",
                    "Hidden.",
                    Place::Free {
                        pose: tomori_protocol::Pose::upright_facing(
                            behind.to_array(),
                            Vec3::new(2.0, 0.4, 0.0).to_array(),
                        ),
                    },
                )],
            );
            let floor = Surface::rectangle(Vec3::ZERO, Vec2::splat(40.0));
            let racks = Surface::rectangle(Vec3::new(0.0, -4.5, 0.0), Vec2::splat(40.0));
            let mut profile = Locomotion::teleport(vec![floor, racks]);
            if pad {
                profile = profile.with_pads([[
                    Pad::at(Vec3::new(2.0, 0.0, -2.0)),
                    Pad::at(Vec3::new(3.0, -4.5, -2.0)),
                ]]);
            }
            conversation.set_locomotion(vec![(crate::test_support::SKY, profile)]);
            let mut step = Step::default();
            let mut feedback = felt();
            grow(&mut step, &mut conversation, &held(0.0), now);
            let frame = step.update(&held(0.0), &mut conversation, &mut feedback, now, FRAME);
            assert_eq!(frame.pads.first().is_some_and(|pad| pad.highlighted), pad);
            step.update(&held(1.0), &mut conversation, &mut feedback, now, FRAME);
            // Without the pad, the grip pulls the panel off; with it nearer, it takes nothing.
            assert_eq!(step.projector.held(), (!pad).then_some("panel-1"), "{pad}");
        }
    }

    #[test]
    fn a_panel_carried_travels_in_the_hand_through_a_teleport_a_turn_and_a_backstep() {
        use crate::locomotion::{Locomotion, Surface};
        use tomori_protocol::{ClientMessage, Panel, Place};
        let mut conversation = lobby(ConnectionRole::Owner);
        conversation.set_locomotion(vec![(
            crate::test_support::SKY,
            Locomotion::teleport(vec![Surface::rectangle(Vec3::ZERO, Vec2::splat(40.0))]),
        )]);
        let ahead = arrival::HOME + Vec3::new(0.0, 0.0, -2.0);
        fixtures::show_projection(
            &mut conversation,
            Vec::new(),
            vec![Panel::new(
                "panel-1",
                "Build status",
                "Green.",
                Place::Free {
                    pose: tomori_protocol::Pose::upright_facing(
                        ahead.to_array(),
                        arrival::HOME.to_array(),
                    ),
                },
            )],
        );
        let mut step = Step::default();
        let mut feedback = felt();
        let now = Instant::now();
        // The platform places the frame's poses with the rig, as it was settled before the frame.
        let placed = |step: &mut Step, conversation: &Conversation, ray: Vec3, grip: f32| {
            let rig = step.rig(conversation).matrix();
            let head = rig * Mat4::from_translation(arrival::HOME);
            Senses {
                beam: true,
                head: Some(head),
                head_pose: Pose::from_transform(head),
                viewer: Some(head.w_axis.truncate()),
                ray: Ray::new(
                    rig.transform_point3(arrival::HOME),
                    rig.transform_vector3(ray),
                ),
                grip: Some(grip),
                ..senses()
            }
        };
        let stick = |step: &mut Step, conversation: &mut Conversation, stick: Vec2| {
            step.buttons(
                &press(|buttons| buttons.left_stick = stick),
                true,
                conversation,
                &mut felt(),
                now,
            );
        };
        let carried = |frame: &Frame<'_>| {
            frame
                .projection
                .iter()
                .find(|sheet| {
                    sheet
                        .canvas
                        .as_deref()
                        .unwrap()
                        .plain()
                        .starts_with("Build")
                })
                .map(|sheet| sheet.model.unwrap().w_axis.truncate())
                .unwrap()
        };
        // Once the scenery has grown in, the panel is pulled off.
        for _ in 0..400 {
            let senses = placed(&mut step, &conversation, -Vec3::Z, 0.0);
            step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
        }
        let senses = placed(&mut step, &conversation, -Vec3::Z, 1.0);
        step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
        assert_eq!(step.projector.held(), Some("panel-1"));
        // Aimed at the floor, the teleport aims, and the panel stays in the hand.
        stick(&mut step, &mut conversation, Vec2::Y);
        let down = Vec3::new(0.0, -0.5, -1.0);
        let senses = placed(&mut step, &conversation, down, 1.0);
        let frame = step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
        assert!(frame.landing.is_some());
        let beam = frame
            .arrival
            .as_ref()
            .and_then(|arrival| arrival.ray)
            .unwrap();
        assert!(beam.hit && beam.end.distance(frame.landing.unwrap()) < 1e-4);
        assert!(carried(&frame).distance(senses.ray.unwrap().at(2.0)) < 1e-3);
        assert_eq!(step.projector.held(), Some("panel-1"));
        // Released, it lands, and the panel comes along: next frame it is at the same place in the hand.
        stick(&mut step, &mut conversation, Vec2::ZERO);
        let frame = step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
        assert_eq!(frame.blink, 1.0);
        assert_ne!(step.rig(&conversation), Rig::default());
        let senses = placed(&mut step, &conversation, -Vec3::Z, 1.0);
        let frame = step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
        assert!(carried(&frame).distance(senses.ray.unwrap().at(2.0)) < 1e-3);
        assert!(carried(&frame).distance(ahead) > 1.0, "carried along");
        // Turned and stepped back, it stays in the hand too.
        for push in [Vec2::X, Vec2::NEG_Y] {
            let before = step.rig(&conversation);
            stick(&mut step, &mut conversation, push);
            step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
            stick(&mut step, &mut conversation, Vec2::ZERO);
            let frame = step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
            // A backstep darkens the view as a teleport does; a turn leaves it coming back.
            assert_eq!(frame.blink == 1.0, push == Vec2::NEG_Y, "{push}");
            assert_ne!(step.rig(&conversation), before, "{push}");
            let senses = placed(&mut step, &conversation, -Vec3::Z, 1.0);
            let frame = step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
            assert!(carried(&frame).distance(senses.ray.unwrap().at(2.0)) < 1e-3);
            assert_eq!(step.projector.held(), Some("panel-1"));
        }
        assert!(conversation.take_actions().is_empty(), "nothing put down");
        // Let go, it floats where the participant carried it to, for everyone.
        let senses = placed(&mut step, &conversation, -Vec3::Z, 0.0);
        step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
        let actions = conversation.take_actions();
        let [
            Action::Send(ClientMessage::PlacePanel {
                pose: Some(pose), ..
            }),
        ] = actions.as_slice()
        else {
            unreachable!("the panel is placed, not {actions:?}")
        };
        let left = Vec3::from_array(pose.position);
        assert!(left.distance(senses.ray.unwrap().at(2.0)) < 1e-3);
        assert!(left.distance(ahead) > 1.0);
    }

    #[test]
    fn the_trigger_held_while_a_wall_is_carried_attracts_no_swarm_and_lets_it_go_unsnapped() {
        use tomori_protocol::{Pose, Wall, WallKind};
        let wall = Wall::new(
            "wall-1",
            WallKind::Added,
            Pose::upright_facing([0.0, 1.5, -2.0], arrival::HOME.to_array()),
        );
        let board = Vec3::new(0.0, 1.5 + 0.42, -2.0);
        let mut conversation = lobby(ConnectionRole::Owner);
        fixtures::show_editable_walls(&mut conversation, vec![wall], Vec::new());
        let mut step = Step::default();
        let now = Instant::now();
        let mut senses = Senses {
            beam: true,
            ray: Ray::new(arrival::HOME, board - arrival::HOME),
            grip: Some(0.0),
            ..senses()
        };
        for _ in 0..400 {
            step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        }
        senses.grip = Some(1.0);
        step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        assert_eq!(step.projector.carried_wall(), Some("wall-1"));
        // Pulled while it is carried, the trigger attracts nothing, and the hint says it is left where it is.
        senses.trigger = Some(1.0);
        senses.attractor = Some(board);
        for _ in 0..3 {
            let frame = step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
            assert!(!frame.attracting);
            assert!(frame.projection.iter().any(|sheet| {
                sheet
                    .canvas
                    .as_deref()
                    .is_some_and(|canvas| canvas.plain().contains("leave it here"))
            }));
        }
        // Let go, it is no longer carried, and the trigger attracts the swarm again.
        senses.grip = Some(0.0);
        step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        assert_eq!(step.projector.carried_wall(), None);
        senses.ray = Ray::new(arrival::HOME, Vec3::Z);
        senses.trigger = Some(0.0);
        step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        senses.trigger = Some(1.0);
        let frame = step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        assert!(!frame.attracting);
        // A change to it the server refused, as made against a version since changed, the frame step takes, to draw
        // the wall as the server has it at once.
        conversation.apply(tomori_client::Event::Message(
            tomori_protocol::ServerMessage::Error {
                error: "That wall changed as you changed it; look again.".into(),
                space: None,
                panel: None,
                wall: Some("wall-1".into()),
            },
        ));
        step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        assert_eq!(conversation.take_refused_wall(), None);
    }

    #[test]
    fn a_wall_is_carried_with_the_grip_put_down_where_let_go_and_removed_by_an_owner_s_b_asked_twice_with_panels()
     {
        use tomori_protocol::{ClientMessage, Panel, Place, Pose, Wall, WallKind};
        let wall = Wall::new(
            "wall-1",
            WallKind::Added,
            Pose::upright_facing([0.0, 1.5, -2.0], arrival::HOME.to_array()),
        );
        let panel = Panel::new(
            "panel-1",
            "Build",
            "Green.",
            Place::Wall {
                wall: "wall-1".into(),
                slot: 2,
            },
        );
        // Between the top row of slots and the label, on the board.
        let board = Vec3::new(0.0, 1.5 + 0.42, -2.0);
        let run = |role: ConnectionRole, editable: bool| {
            let mut conversation = lobby(role);
            if editable {
                fixtures::show_editable_walls(
                    &mut conversation,
                    vec![wall.clone()],
                    vec![panel.clone()],
                );
            } else {
                fixtures::show_projection(
                    &mut conversation,
                    vec![wall.clone()],
                    vec![panel.clone()],
                );
            }
            let mut step = Step::default();
            let start = Instant::now();
            let mut senses = Senses {
                beam: true,
                ray: Ray::new(arrival::HOME, board - arrival::HOME),
                grip: Some(0.0),
                ..senses()
            };
            for _ in 0..400 {
                step.update(&senses, &mut conversation, &mut felt(), start, FRAME);
            }
            senses.grip = Some(1.0);
            let frame = step.update(&senses, &mut conversation, &mut felt(), start, FRAME);
            let hint = frame
                .projection
                .iter()
                .filter_map(|sheet| sheet.canvas.as_deref().map(crate::canvas::Canvas::plain))
                .find(|text| text.contains("leave it here"));
            (conversation, step, senses, hint, start)
        };
        let b = |step: &mut Step, conversation: &mut Conversation, at: Instant| {
            for (back, held, after) in [(true, true, 0), (false, false, 100)] {
                step.buttons(
                    &press(|buttons| {
                        buttons.back = back;
                        buttons.back_held = held;
                    }),
                    true,
                    conversation,
                    &mut felt(),
                    at + Duration::from_millis(after),
                );
            }
        };
        let removals = |conversation: &mut Conversation| {
            conversation
                .take_actions()
                .into_iter()
                .filter(|action| matches!(action, Action::Send(ClientMessage::RemoveWall { .. })))
                .count()
        };
        // An owner carries it, and is told B removes it.
        let (mut conversation, mut step, mut senses, hint, start) =
            run(ConnectionRole::Owner, true);
        assert_eq!(step.projector.carried_wall(), Some("wall-1"));
        let hint = hint.expect("the hint shows while carrying");
        assert!(hint.contains("remove, for all"), "{hint}");
        // It holds a panel, so a first press of B only asks again, and a second removes it, for everyone.
        b(&mut step, &mut conversation, start);
        assert_eq!(removals(&mut conversation), 0);
        b(
            &mut step,
            &mut conversation,
            start + Duration::from_millis(500),
        );
        assert_eq!(removals(&mut conversation), 1);
        assert_eq!(step.projector.editing_wall(), None);
        // Carried elsewhere and let go, its new place is sent.
        let (mut conversation, mut step, _, _, start) = run(ConnectionRole::Owner, true);
        senses.ray = Ray::new(
            arrival::HOME,
            board + Vec3::new(-0.8, 0.0, 0.0) - arrival::HOME,
        );
        step.update(&senses, &mut conversation, &mut felt(), start, FRAME);
        senses.grip = Some(0.0);
        step.update(&senses, &mut conversation, &mut felt(), start, FRAME);
        let sent = conversation.take_actions();
        assert!(
            sent.iter().any(|action| matches!(
                action,
                Action::Send(ClientMessage::SetWall { wall, pose: Some(pose), grid: None, .. })
                    if wall == "wall-1" && pose.position[0] < -0.5
            )),
            "{sent:?}"
        );
        // Held, not carried, B lets go of it, and backs out of nothing else.
        assert_eq!(step.projector.editing_wall(), Some("wall-1"));
        b(&mut step, &mut conversation, start);
        assert_eq!(step.projector.editing_wall(), None);
        assert!(conversation.take_actions().is_empty());
        // A collaborator carries it, but B removes nothing, and their hint does not offer it.
        let (mut conversation, mut step, _, hint, start) = run(ConnectionRole::Collaborator, true);
        assert_eq!(step.projector.carried_wall(), Some("wall-1"));
        assert!(!hint.unwrap().contains("remove"));
        b(&mut step, &mut conversation, start);
        b(
            &mut step,
            &mut conversation,
            start + Duration::from_millis(500),
        );
        assert_eq!(removals(&mut conversation), 0);
        // A visitor, and anyone where the server takes no wall changes, takes no wall.
        let (_, step, _, _, _) = run(ConnectionRole::Visitor, true);
        assert_eq!(step.projector.editing_wall(), None);
        let (mut conversation, step, _, _, _) = run(ConnectionRole::Owner, false);
        assert_eq!(step.projector.editing_wall(), None);
        let change = crate::projection::editing::Change {
            wall: "wall-1".into(),
            grid: None,
            scale: Some(1.5),
            aspect: None,
            shape: None,
            pose: None,
            join: None,
            tilt: None,
            version: Some(7),
        };
        assert!(!conversation.set_wall(change.clone()));
        assert!(!conversation.remove_wall_by_id("wall-1".into()));
        assert!(conversation.take_actions().is_empty());
        let mut conversation = lobby(ConnectionRole::Collaborator);
        fixtures::show_editable_walls(&mut conversation, vec![wall.clone()], Vec::new());
        assert!(conversation.set_wall(change.clone()));
        assert!(!conversation.remove_wall_by_id("wall-1".into()));
        // Made against the wall as it was taken hold of, it says its version then, so that the server refuses it where
        // the wall changed since; one made against none, or a wall from an older server, at 0, says none.
        assert!(matches!(
            conversation.take_actions().as_slice(),
            [Action::Send(ClientMessage::SetWall {
                revision: Some(7),
                ..
            })]
        ));
        for version in [None, Some(0)] {
            assert!(conversation.set_wall(crate::projection::editing::Change {
                version,
                ..change.clone()
            }));
            assert!(matches!(
                conversation.take_actions().as_slice(),
                [Action::Send(ClientMessage::SetWall { revision: None, .. })]
            ));
        }
        // A snap is sent as one, and so is taking a wall out of its group, where it was let go.
        use crate::projection::editing::Joining;
        let attach = tomori_protocol::Attach::new(
            "wall-2",
            tomori_protocol::Edge::Right,
            tomori_protocol::Align::Middle,
        );
        conversation.take_actions();
        let pose = tomori_protocol::Pose::default();
        assert!(conversation.set_wall(crate::projection::editing::Change {
            join: Some(Joining::Attach {
                attach: attach.clone(),
                alone: true,
            }),
            ..change.clone()
        }));
        assert!(conversation.set_wall(crate::projection::editing::Change {
            join: Some(Joining::Leave),
            pose: Some(pose),
            ..change.clone()
        }));
        assert_eq!(
            conversation.take_actions(),
            [
                Action::Send(ClientMessage::AttachWall {
                    wall: "wall-1".into(),
                    attach: Some(attach.clone()),
                    alone: true,
                    pose: None,
                }),
                Action::Send(ClientMessage::AttachWall {
                    wall: "wall-1".into(),
                    attach: None,
                    alone: true,
                    pose: Some(pose),
                }),
            ]
        );
        // A server that snaps no walls is sent neither.
        conversation.apply(fixtures::space(|state| {
            state.projection = true;
            state.walls = true;
            state.groups = false;
        }));
        assert!(!conversation.wall_rights().snap);
        assert!(!conversation.set_wall(crate::projection::editing::Change {
            join: Some(Joining::Leave),
            ..change
        }));
    }

    #[test]
    fn an_owner_unlocks_a_locked_wall_with_its_padlock_for_everyone_while_others_only_see_it_locked()
     {
        use crate::projection::editing::{LockChange, LockControls, handle_points_with};
        use tomori_protocol::{ClientMessage, Pose, Wall, WallKind};
        let wall = Wall::new(
            "environment-0",
            WallKind::Environment,
            Pose::upright_facing([0.0, 1.5, -2.0], arrival::HOME.to_array()),
        )
        .with_grid(tomori_protocol::Grid::from((2, 1)))
        .with_locked(true);
        let board = Vec3::new(0.0, 1.5 + 0.2, -2.0);
        let frame_of = crate::board::model(&wall.pose).unwrap();
        let lock = LockControls {
            padlock: true,
            reset: false,
        };
        let (points, _) = handle_points_with(&wall, frame_of, arrival::HOME, lock);
        let padlock = points[0].1;
        let run = |role: ConnectionRole, locks: bool| {
            let mut conversation = lobby(role);
            if locks {
                fixtures::show_lockable_walls(&mut conversation, vec![wall.clone()], Vec::new());
            } else {
                fixtures::show_editable_walls(&mut conversation, vec![wall.clone()], Vec::new());
            }
            let mut step = Step::default();
            let start = Instant::now();
            let mut senses = Senses {
                beam: true,
                ray: Ray::new(arrival::HOME, board - arrival::HOME),
                grip: Some(0.0),
                ..senses()
            };
            for _ in 0..400 {
                step.update(&senses, &mut conversation, &mut felt(), start, FRAME);
            }
            // Taken hold of by its board, it is held, not carried.
            for grip in [1.0, 0.0] {
                senses.grip = Some(grip);
                step.update(&senses, &mut conversation, &mut felt(), start, FRAME);
            }
            assert_eq!(step.projector.editing_wall(), Some("environment-0"));
            assert_eq!(step.projector.carried_wall(), None);
            // Squeezed, its padlock unlocks it, for an owner.
            senses.ray = Ray::new(arrival::HOME, padlock - arrival::HOME);
            let mut hint = None;
            for grip in [0.0, 1.0, 0.0] {
                senses.grip = Some(grip);
                let frame = step.update(&senses, &mut conversation, &mut felt(), start, FRAME);
                hint = hint.or_else(|| {
                    frame
                        .projection
                        .iter()
                        .filter_map(|sheet| {
                            sheet.canvas.as_deref().map(crate::canvas::Canvas::plain)
                        })
                        .find(|text| text.contains("done"))
                });
            }
            (conversation.take_actions(), hint)
        };
        let unlocks = |sent: &[Action]| {
            sent.iter()
                .filter(|action| {
                    matches!(
                        action,
                        Action::Send(ClientMessage::LockWall { wall, locked: false, reset: false })
                            if wall == "environment-0"
                    )
                })
                .count()
        };
        let (sent, hint) = run(ConnectionRole::Owner, true);
        assert_eq!(unlocks(&sent), 1, "{sent:?}");
        assert!(hint.unwrap().contains("unlock it to change it"));
        // A collaborator sees it locked, told an owner can unlock it, and sends nothing.
        let (sent, hint) = run(ConnectionRole::Collaborator, true);
        assert_eq!(unlocks(&sent), 0, "{sent:?}");
        assert!(sent.is_empty(), "{sent:?}");
        assert!(hint.unwrap().contains("an owner can unlock it"));
        // Where the server locks no walls, as an older one, there is no padlock to squeeze, and nothing is sent.
        let (sent, _) = run(ConnectionRole::Owner, false);
        assert_eq!(unlocks(&sent), 0, "{sent:?}");
        // Nor does anyone ask a server that locks no walls, and only an owner asks one that does.
        let mut conversation = lobby(ConnectionRole::Owner);
        fixtures::show_editable_walls(&mut conversation, vec![wall.clone()], Vec::new());
        let change = LockChange {
            wall: "environment-0".into(),
            locked: false,
            reset: false,
        };
        assert!(!conversation.lock_wall(change.clone()));
        let mut conversation = lobby(ConnectionRole::Collaborator);
        fixtures::show_lockable_walls(&mut conversation, vec![wall.clone()], Vec::new());
        assert!(!conversation.lock_wall(change.clone()));
        assert!(conversation.take_actions().is_empty());
        let mut conversation = lobby(ConnectionRole::Owner);
        fixtures::show_lockable_walls(&mut conversation, vec![wall.clone()], Vec::new());
        assert!(conversation.lock_wall(change));
    }

    #[test]
    fn b_throws_a_panel_away_only_while_it_is_carried_and_a_hold_then_neither_throws_nor_leaves() {
        use tomori_protocol::{ClientMessage, Panel, Place};
        let mut conversation = lobby(ConnectionRole::Owner);
        let ahead = arrival::HOME + Vec3::new(0.0, 0.0, -2.0);
        fixtures::show_projection(
            &mut conversation,
            Vec::new(),
            vec![Panel::new(
                "panel-1",
                "Build status",
                "Green.",
                Place::Free {
                    pose: tomori_protocol::Pose::upright_facing(
                        ahead.to_array(),
                        arrival::HOME.to_array(),
                    ),
                },
            )],
        );
        let mut step = Step::default();
        let mut feedback = felt();
        let start = Instant::now();
        let mut senses = Senses {
            beam: true,
            ray: Ray::new(arrival::HOME, -Vec3::Z),
            grip: Some(0.0),
            ..senses()
        };
        for _ in 0..400 {
            step.update(&senses, &mut conversation, &mut feedback, start, FRAME);
        }
        let hint = |frame: &Frame<'_>| {
            frame
                .projection
                .iter()
                .map(|sheet| sheet.canvas.as_deref().unwrap().plain())
                .find(|text| text.contains("throw away"))
        };
        let frame = |step: &mut Step, conversation: &mut Conversation, senses: &Senses, at| {
            let frame = step.update(senses, conversation, &mut felt(), at, FRAME);
            (
                hint(&frame),
                frame.projection.iter().any(|sheet| {
                    sheet
                        .canvas
                        .as_deref()
                        .unwrap()
                        .plain()
                        .starts_with("Build")
                }),
            )
        };
        let b = |step: &mut Step, conversation: &mut Conversation, back: bool, held: bool, at| {
            step.buttons(
                &press(|buttons| {
                    buttons.back = back;
                    buttons.back_held = held;
                }),
                true,
                conversation,
                &mut felt(),
                at,
            );
        };
        // Pointed at, not carried, a press of B dismisses nothing, and no hint shows.
        let (shown, _) = frame(&mut step, &mut conversation, &senses, start);
        assert_eq!(shown, None);
        assert_eq!(
            step.update(&senses, &mut conversation, &mut felt(), start, FRAME)
                .presence
                .and_then(|presence| presence.carrying),
            None
        );
        b(&mut step, &mut conversation, true, true, start);
        b(
            &mut step,
            &mut conversation,
            false,
            false,
            start + Duration::from_millis(100),
        );
        assert!(conversation.take_actions().is_empty());
        assert_eq!(conversation.notice(), Some(crate::conversation::THROW_HOW));
        // Carried with the grip, the hint shows beside it.
        senses.grip = Some(1.0);
        frame(&mut step, &mut conversation, &senses, start);
        let (shown, _) = frame(&mut step, &mut conversation, &senses, start);
        assert_eq!(step.projector.held(), Some("panel-1"));
        // The server is told which panel is carried, so that the agent leaves it in the hand.
        let carrying = |step: &mut Step, conversation: &mut Conversation, senses: &Senses| {
            step.update(senses, conversation, &mut felt(), start, FRAME)
                .presence
                .and_then(|presence| presence.carrying)
        };
        assert_eq!(
            carrying(&mut step, &mut conversation, &senses).as_deref(),
            Some("panel-1")
        );
        let shown = shown.expect("the hint shows while carrying");
        assert!(shown.contains("throw away, for all"), "{shown}");
        assert!(shown.contains("leave it floating"), "{shown}");
        // Held while carrying, B neither throws the panel away nor leaves, however long, and letting it go then does
        // nothing.
        let now = start + Duration::from_secs(1);
        b(&mut step, &mut conversation, true, true, now);
        for held in [LEAVE_HOLD / 2, LEAVE_HOLD, LEAVE_HOLD * 2] {
            b(&mut step, &mut conversation, false, true, now + held);
            frame(&mut step, &mut conversation, &senses, now + held);
        }
        b(
            &mut step,
            &mut conversation,
            false,
            false,
            now + LEAVE_HOLD * 2,
        );
        assert!(conversation.take_actions().is_empty());
        assert!(conversation.joined_space().is_some(), "still in the space");
        assert_eq!(step.projector.held(), Some("panel-1"));
        // Let go of the grip before B, the panel is put down, not thrown away.
        let now = now + LEAVE_HOLD * 3;
        b(&mut step, &mut conversation, true, true, now);
        senses.grip = Some(0.0);
        frame(&mut step, &mut conversation, &senses, now);
        b(
            &mut step,
            &mut conversation,
            false,
            false,
            now + Duration::from_millis(50),
        );
        let actions = conversation.take_actions();
        assert!(
            matches!(
                actions.as_slice(),
                [Action::Send(ClientMessage::PlacePanel { .. })]
            ),
            "{actions:?}"
        );
        // Picked up again, a press of B, let go soon enough, throws it away for everyone at once.
        let now = now + Duration::from_secs(1);
        senses.grip = Some(1.0);
        frame(&mut step, &mut conversation, &senses, now + PENDING_OVER);
        assert_eq!(step.projector.held(), Some("panel-1"));
        let now = now + PENDING_OVER;
        b(&mut step, &mut conversation, true, true, now);
        assert!(
            conversation.take_actions().is_empty(),
            "only once B is let go"
        );
        b(
            &mut step,
            &mut conversation,
            false,
            false,
            now + Duration::from_millis(200),
        );
        let actions = conversation.take_actions();
        assert_eq!(
            actions,
            [Action::Send(ClientMessage::DismissPanel {
                panel: "panel-1".into()
            })]
        );
        assert_eq!(step.projector.held(), None);
        // It is neither drawn nor hinted at, nor put down as the grip is let go, while the server agrees.
        let (shown, drawn) = frame(&mut step, &mut conversation, &senses, now);
        assert_eq!((shown, drawn), (None, false));
        senses.grip = Some(0.0);
        frame(&mut step, &mut conversation, &senses, now);
        assert!(conversation.take_actions().is_empty());
        // Holding B, with nothing carried, leaves the space as ever.
        let now = now + Duration::from_secs(1);
        b(&mut step, &mut conversation, true, true, now);
        b(&mut step, &mut conversation, false, true, now + LEAVE_HOLD);
        assert!(conversation.joined_space().is_none(), "left");
    }

    /// Longer than a panel let go is shown where it was let go, so that the server's projection, which these tests do
    /// not change, stands it again.
    const PENDING_OVER: Duration = Duration::from_secs(4);

    #[test]
    fn a_panel_carried_takes_the_right_stick_and_its_click_from_the_transcript_and_replays() {
        use crate::locomotion::{Locomotion, Surface};
        use tomori_protocol::{ClientMessage, Panel, Place, ServerMessage};
        let mut conversation = conversed();
        conversation.set_locomotion(vec![(
            crate::test_support::SKY,
            Locomotion::teleport(vec![Surface::rectangle(Vec3::ZERO, Vec2::splat(40.0))]),
        )]);
        let messages = conversation.joined_space().unwrap().0.messages.clone();
        conversation.apply(space(|state| {
            state.messages = messages;
            state.projection = true;
            state.speech = tomori_protocol::SpeechSetting::Full;
        }));
        let ahead = arrival::HOME + Vec3::new(0.0, 0.0, -2.0);
        conversation.apply(tomori_client::Event::Message(ServerMessage::Projection {
            projection: Box::new(
                tomori_protocol::Projection::new(SPACE, "sky")
                    .with_walls(vec![tomori_wall()])
                    .with_panels(vec![Panel::new(
                        "panel-1",
                        "Build status",
                        "Green.",
                        Place::Free {
                            pose: tomori_protocol::Pose::upright_facing(
                                ahead.to_array(),
                                arrival::HOME.to_array(),
                            ),
                        },
                    )]),
            ),
        }));
        conversation.take_actions();
        let mut step = Step::default();
        let mut feedback = felt();
        let start = Instant::now();
        let mut senses = Senses {
            ray: Ray::new(arrival::HOME, -Vec3::Z),
            right_palm: Some(palm_up(0.0)),
            grip: Some(0.0),
            ..senses()
        };
        // The transcript shows on the upturned palm, then goes as the palm turns away, and the panel is pulled off.
        assert!(
            wrist_after(&mut step, &mut conversation, &senses, start)
                .model
                .is_some()
        );
        senses.right_palm = None;
        for _ in 0..400 {
            step.update(
                &senses,
                &mut conversation,
                &mut feedback,
                start + HOLD,
                FRAME,
            );
        }
        senses.grip = Some(1.0);
        step.update(
            &senses,
            &mut conversation,
            &mut feedback,
            start + HOLD,
            FRAME,
        );
        assert_eq!(step.projector.held(), Some("panel-1"));
        // Carried, the upturned palm shows no transcript, which would let the panel go.
        senses.right_palm = Some(palm_up(0.0));
        let wrist = wrist_after(&mut step, &mut conversation, &senses, start + HOLD * 2);
        assert!(wrist.model.is_none());
        assert_eq!(step.projector.held(), Some("panel-1"));
        let now = start + HOLD * 3;
        let carried = |frame: &Frame<'_>| {
            frame
                .projection
                .iter()
                .find(|sheet| {
                    sheet
                        .canvas
                        .as_deref()
                        .unwrap()
                        .plain()
                        .starts_with("Build")
                })
                .map(|sheet| sheet.model.unwrap())
                .unwrap()
        };
        let buttons = |step: &mut Step, conversation: &mut Conversation, buttons: Buttons| {
            step.buttons(&buttons, true, conversation, &mut felt(), now);
        };
        // A flick of the right stick turns the panel a snap, with a tick, and moves neither the transcript's focus nor the
        // cards; the left stick turns the participant in the same frame.
        let focus = conversation.wrist_focus().cloned();
        let square = carried(&step.update(&senses, &mut conversation, &mut feedback, now, FRAME));
        buttons(
            &mut step,
            &mut conversation,
            press(|buttons| {
                buttons.right_stick = Vec2::X;
                buttons.left_stick = Vec2::NEG_X;
            }),
        );
        felt_on(&mut feedback, Side::Right, now);
        let frame = step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
        let turned = carried(&frame);
        assert!(
            (square
                .z_axis
                .truncate()
                .angle_between(turned.z_axis.truncate())
                - crate::manipulation::YAW_STEP)
                .abs()
                < 1e-3
        );
        assert!(!felt_on(&mut feedback, Side::Right, now).is_empty());
        assert_eq!(conversation.wrist_focus().cloned(), focus);
        assert_eq!(step.presentation_navigation.card, 0);
        assert!(
            step.rig(&conversation)
                .matrix()
                .transform_vector3(-Vec3::Z)
                .x
                < -0.4
        );
        // Its click turns it back square, and plays nothing again.
        buttons(
            &mut step,
            &mut conversation,
            press(|buttons| buttons.replay = true),
        );
        let frame = step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
        let straightened = carried(&frame);
        assert!(
            straightened
                .z_axis
                .truncate()
                .angle_between(square.z_axis.truncate())
                < 1e-3
        );
        assert!(conversation.take_actions().is_empty(), "no replay");
        // A push begun while carrying stays the panel's after it is let go, so the transcript then showing takes none of it.
        buttons(
            &mut step,
            &mut conversation,
            press(|buttons| buttons.right_stick = Vec2::NEG_Y),
        );
        senses.grip = Some(0.0);
        step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
        assert_eq!(step.projector.held(), None);
        conversation.take_actions();
        assert!(
            wrist_after(&mut step, &mut conversation, &senses, now)
                .model
                .is_some()
        );
        for _ in 0..3 {
            buttons(
                &mut step,
                &mut conversation,
                press(|buttons| buttons.right_stick = Vec2::NEG_Y),
            );
        }
        assert_eq!(conversation.wrist_focus().cloned(), focus);
        // Back at the middle, the next push is the transcript's, and the click replays again.
        buttons(&mut step, &mut conversation, Buttons::default());
        buttons(
            &mut step,
            &mut conversation,
            press(|buttons| buttons.right_stick = Vec2::NEG_X),
        );
        assert_ne!(conversation.wrist_focus().cloned(), focus);
        buttons(&mut step, &mut conversation, Buttons::default());
        buttons(
            &mut step,
            &mut conversation,
            press(|buttons| buttons.replay = true),
        );
        assert!(matches!(
            conversation.take_actions().as_slice(),
            [Action::Send(ClientMessage::Replay { message: None })]
        ));
        // Losing focus forgets whose the push was.
        buttons(
            &mut step,
            &mut conversation,
            press(|buttons| buttons.right_stick = Vec2::X),
        );
        step.blur(&mut conversation);
        assert_eq!(step.right_stick, RightStick::Free);
        assert_eq!(
            RightStick::Other.update(Vec2::NAN, || RightStick::Carried),
            RightStick::Free
        );
    }

    #[test]
    fn a_push_of_the_right_stick_pointing_at_a_panel_with_more_than_it_shows_scrolls_it_for_this_participant()
     {
        use tomori_protocol::{Code, Panel, Place};
        let mut conversation = joined();
        // Thirty lines of code in the Tomori wall's middle slot, more than a slot shows.
        let body: Vec<String> = (1..=30)
            .map(|line| format!("let x{line} = {line};"))
            .collect();
        show_projection(
            &mut conversation,
            Vec::new(),
            vec![
                Panel::new(
                    "panel-1",
                    "rust",
                    body.join("\n"),
                    Place::Wall {
                        wall: tomori_protocol::TOMORI_WALL.into(),
                        slot: 2,
                    },
                )
                .with_code(Code::new("rust", 30)),
            ],
        );
        let wall = tomori_wall();
        let frame = Mat4::from_rotation_translation(
            glam::Quat::from_array(wall.pose.orientation),
            Vec3::from_array(wall.pose.position),
        );
        let slot = crate::projection::slot_frames(&wall, frame)[2]
            .w_axis
            .truncate();
        let mut step = Step::default();
        let mut feedback = felt();
        let now = Instant::now();
        let on_panel = Senses {
            ray: Ray::new(arrival::HOME, slot - arrival::HOME),
            grip: Some(0.0),
            ..senses()
        };
        let away = Senses {
            ray: Ray::new(arrival::HOME, Vec3::Y),
            ..on_panel
        };
        let footer = |frame: &Frame<'_>| {
            frame
                .projection
                .iter()
                .filter_map(|sheet| sheet.canvas.as_deref())
                .find(|canvas| canvas.plain().starts_with("rust"))
                .map(|canvas| canvas.plain().lines().last().unwrap().trim_end().to_owned())
                .unwrap()
        };
        let buttons = |step: &mut Step, conversation: &mut Conversation, buttons: Buttons| {
            step.buttons(&buttons, true, conversation, &mut felt(), now);
        };
        // Pointed at, its label says the stick scrolls it.
        let frame = step.update(&on_panel, &mut conversation, &mut feedback, now, FRAME);
        assert_eq!(footer(&frame), "↓ 14 more");
        assert!(
            frame
                .projection
                .iter()
                .filter_map(|sheet| sheet.canvas.as_deref())
                .any(|canvas| canvas.plain().contains("Stick ↕  scroll"))
        );
        conversation.take_actions();
        // Pulled back, the stick scrolls it down, faster the further it is pulled, paging no cards and sending nothing.
        buttons(
            &mut step,
            &mut conversation,
            press(|buttons| buttons.right_stick = Vec2::NEG_Y),
        );
        assert!(matches!(step.right_stick, RightStick::Scrolled(ref panel) if panel == "panel-1"));
        step.update(&on_panel, &mut conversation, &mut feedback, now, 0.2);
        assert_eq!(step.projector.scroll("panel-1"), 3);
        assert_eq!(step.presentation_navigation.card, 0);
        assert!(conversation.take_actions().is_empty());
        // The push stays the panel's while the ray wanders off it, until the stick comes back to the middle.
        step.update(&away, &mut conversation, &mut feedback, now, 10.0);
        assert_eq!(step.projector.scroll("panel-1"), 14);
        let frame = step.update(&on_panel, &mut conversation, &mut feedback, now, FRAME);
        assert_eq!(footer(&frame), "↑ 14 above");
        buttons(&mut step, &mut conversation, Buttons::default());
        assert_eq!(step.right_stick, RightStick::Free);
        // Pushed forward, it scrolls back up.
        step.update(&on_panel, &mut conversation, &mut feedback, now, FRAME);
        buttons(
            &mut step,
            &mut conversation,
            press(|buttons| buttons.right_stick = Vec2::Y),
        );
        step.update(&on_panel, &mut conversation, &mut feedback, now, 0.4);
        assert_eq!(step.projector.scroll("panel-1"), 8);
        buttons(&mut step, &mut conversation, Buttons::default());
        // A push begun pointing at nothing that scrolls is not the panel's, even once the ray comes back to it.
        step.update(&away, &mut conversation, &mut feedback, now, FRAME);
        buttons(
            &mut step,
            &mut conversation,
            press(|buttons| buttons.right_stick = Vec2::NEG_Y),
        );
        assert_eq!(step.right_stick, RightStick::Other);
        step.update(&on_panel, &mut conversation, &mut feedback, now, 1.0);
        assert_eq!(step.projector.scroll("panel-1"), 8);
        buttons(&mut step, &mut conversation, Buttons::default());
        // The wheel scrolls the panel pointed at, by its rows, at once.
        step.update(&on_panel, &mut conversation, &mut feedback, now, FRAME);
        buttons(
            &mut step,
            &mut conversation,
            press(|buttons| buttons.scroll = -3.0),
        );
        buttons(
            &mut step,
            &mut conversation,
            press(|buttons| buttons.scroll = -2.0),
        );
        step.update(&on_panel, &mut conversation, &mut feedback, now, FRAME);
        assert_eq!(step.projector.scroll("panel-1"), 3);
        // A push begun scrolling, then the grip pulling the panel off, scrolls it no further and does not push it.
        buttons(
            &mut step,
            &mut conversation,
            press(|buttons| buttons.right_stick = Vec2::Y),
        );
        let gripped = Senses {
            grip: Some(1.0),
            ..on_panel
        };
        step.update(&gripped, &mut conversation, &mut feedback, now, FRAME);
        assert_eq!(step.projector.held(), Some("panel-1"));
        let carried = |frame: &Frame<'_>| {
            frame
                .projection
                .iter()
                .find(|sheet| sheet.canvas.as_deref().unwrap().plain().starts_with("rust"))
                .map(|sheet| sheet.model.unwrap().w_axis.truncate())
                .unwrap()
        };
        let before = carried(&step.update(&gripped, &mut conversation, &mut feedback, now, FRAME));
        buttons(
            &mut step,
            &mut conversation,
            press(|buttons| buttons.right_stick = Vec2::Y),
        );
        assert_eq!(step.right_stick, RightStick::Scrolled("panel-1".into()));
        let after = carried(&step.update(&gripped, &mut conversation, &mut feedback, now, 0.5));
        assert!(before.distance(after) < 1e-4);
        // Carried, a new push moves it, as ever, and the wheel scrolls nothing.
        buttons(&mut step, &mut conversation, Buttons::default());
        buttons(
            &mut step,
            &mut conversation,
            press(|buttons| {
                buttons.right_stick = Vec2::Y;
                buttons.scroll = 3.0;
            }),
        );
        assert_eq!(step.right_stick, RightStick::Carried);
        assert_eq!(step.wheel, None);
        let pushed = carried(&step.update(&gripped, &mut conversation, &mut feedback, now, 0.5));
        assert!(pushed.distance(arrival::HOME) > after.distance(arrival::HOME) + 0.1);
        // Losing focus forgets the scrolling push and the wheel.
        step.blur(&mut conversation);
        assert_eq!(
            (step.scroll_stick.clone(), step.wheel.clone()),
            (None, None)
        );
    }

    #[test]
    fn the_stick_and_the_wheel_scroll_together_the_stick_s_panel_first() {
        let stick = ("panel-1".to_owned(), -1.0);
        let rows = crate::projection::SCROLL_SPEED * 0.5;
        // Both on one panel add; on two, the stick's, until it is let go; either alone, its own.
        assert_eq!(
            scrolled(Some(&stick), Some(("panel-1".into(), 2.0)), 0.5),
            Some(("panel-1".into(), rows + 2.0))
        );
        assert_eq!(
            scrolled(Some(&stick), Some(("panel-2".into(), 2.0)), 0.5),
            Some(("panel-1".into(), rows))
        );
        assert_eq!(
            scrolled(None, Some(("panel-2".into(), 2.0)), 0.5),
            Some(("panel-2".into(), 2.0))
        );
        assert_eq!(scrolled(None, None, 0.5), None);
    }

    #[test]
    fn a_panel_carried_to_the_tomori_wall_shows_its_slot_and_is_put_in_it_for_everyone() {
        use tomori_protocol::{ClientMessage, Panel, Place};
        let mut conversation = lobby(ConnectionRole::Owner);
        let ahead = arrival::HOME + Vec3::new(-1.0, 0.0, -1.5);
        fixtures::show_projection(
            &mut conversation,
            Vec::new(),
            vec![Panel::new(
                "panel-1",
                "Build status",
                "Green.",
                Place::Free {
                    pose: tomori_protocol::Pose::upright_facing(
                        ahead.to_array(),
                        arrival::HOME.to_array(),
                    ),
                },
            )],
        );
        let wall = tomori_wall();
        let frame = crate::board::model(&wall.pose).unwrap();
        let slot = crate::projection::slot_frames(&wall, frame)[3];
        let mut step = Step::default();
        let now = Instant::now();
        let mut senses = Senses {
            beam: true,
            ray: Ray::new(arrival::HOME, ahead - arrival::HOME),
            grip: Some(0.0),
            ..senses()
        };
        step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        senses.grip = Some(1.0);
        step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        assert_eq!(step.projector.held(), Some("panel-1"));
        // Pointed at the wall's fourth slot, its outline shows there, and the ray ends on it.
        let middle = slot.w_axis.truncate();
        senses.ray = Ray::new(arrival::HOME, middle - arrival::HOME);
        let output = step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        let outline = slot * Mat4::from_translation(Vec3::Z * 0.01);
        assert!(
            output
                .projection
                .iter()
                .any(|sheet| sheet.model == Some(outline))
        );
        let beam = output
            .arrival
            .as_ref()
            .and_then(|arrival| arrival.ray)
            .unwrap();
        assert!(beam.end.distance(middle) < 1e-3);
        // Let go, it is put in that slot, for everyone.
        senses.grip = Some(0.0);
        step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        assert_eq!(
            conversation.take_actions(),
            [Action::Send(ClientMessage::PlacePanel {
                panel: "panel-1".into(),
                wall: Some(tomori_protocol::TOMORI_WALL.into()),
                slot: Some(3),
                pose: None,
            })]
        );
    }

    #[test]
    fn pointed_at_on_a_checklist_item_as_x_is_pressed_the_words_name_its_line() {
        use tomori_protocol::{Panel, Place, Pose, Resource, ServerMessage};
        let mut conversation = lobby(ConnectionRole::Owner);
        conversation.apply(space(|state| state.projection = true));
        let ahead = arrival::HOME + Vec3::new(0.0, 0.0, -2.0);
        conversation.apply(tomori_client::Event::Message(ServerMessage::Projection {
            projection: Box::new(
                tomori_protocol::Projection::new(SPACE, "sky")
                    .with_walls(vec![tomori_wall()])
                    .with_panels(vec![Panel::new(
                        "panel-1",
                        "Agenda",
                        "To cover:\n\n- [x] Panel ids\n- [ ] Pairing window",
                        Place::Free {
                            pose: Pose::upright_facing(ahead.to_array(), arrival::HOME.to_array()),
                        },
                    )]),
            ),
        }));
        conversation.take_actions();
        let mut step = Step::default();
        let now = Instant::now();
        // From the top of the panel down, what X held attends to.
        let mut named = Vec::new();
        for step_down in 0..60 {
            let at = ahead + Vec3::new(-0.05, 0.15 - step_down as f32 * 0.005, 0.0);
            let senses = Senses {
                beam: true,
                ray: Ray::new(arrival::HOME, at - arrival::HOME),
                ..senses()
            };
            step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
            conversation.talk(true);
            conversation.talk(false);
            let actions = conversation.take_actions();
            let Some(Action::BeginUtterance(utterance)) = actions.first() else {
                unreachable!("recording begins, not {actions:?}")
            };
            if let Some(attended) = utterance.attention.first()
                && named.last() != Some(attended)
            {
                named.push(attended.clone());
            }
        }
        // The panel itself above its list, then each item by the line it begins on, then the panel again below.
        assert_eq!(
            named,
            [
                Resource::panel("panel-1"),
                Resource::panel_at("panel-1", Some(2)),
                Resource::panel_at("panel-1", Some(3)),
                Resource::panel("panel-1"),
            ]
        );
    }

    /// The senses with the ray on each line of the queue [`show_queue`] floats at `ahead`, from the top down: the first
    /// that meets each of its rows, `pod7`, `pod112` and `pod9`.
    fn queue_rows(step: &mut Step, conversation: &mut Conversation, ahead: Vec3) -> [Senses; 3] {
        let mut found: [Option<Senses>; 3] = [None; 3];
        for step_down in 0..60 {
            let at = ahead + Vec3::new(-0.05, 0.15 - step_down as f32 * 0.005, 0.0);
            let senses = Senses {
                beam: true,
                ray: Ray::new(arrival::HOME, at - arrival::HOME),
                ..senses()
            };
            step.update(&senses, conversation, &mut felt(), Instant::now(), FRAME);
            if let Some((_, line)) = conversation.pointed_item()
                && found[*line as usize].is_none()
            {
                found[*line as usize] = Some(senses);
            }
        }
        found.map(|senses| senses.expect("each row"))
    }

    #[test]
    fn pointed_at_on_a_row_of_an_environment_s_list_the_words_name_the_row_by_its_id() {
        use tomori_protocol::Resource;
        let mut conversation = lobby(ConnectionRole::Owner);
        let ahead = arrival::HOME + Vec3::new(0.0, 0.0, -2.0);
        show_queue(&mut conversation, ahead, arrival::HOME);
        let mut step = Step::default();
        let mut named: Vec<Resource> = Vec::new();
        for step_down in 0..60 {
            let at = ahead + Vec3::new(-0.05, 0.15 - step_down as f32 * 0.005, 0.0);
            let senses = Senses {
                beam: true,
                ray: Ray::new(arrival::HOME, at - arrival::HOME),
                ..senses()
            };
            step.update(
                &senses,
                &mut conversation,
                &mut felt(),
                Instant::now(),
                FRAME,
            );
            conversation.talk(true);
            conversation.talk(false);
            // Recording begins, and ends, attending to what is pointed at.
            let attended: Vec<_> = conversation
                .take_actions()
                .into_iter()
                .filter_map(|action| match action {
                    Action::BeginUtterance(utterance) => Some(utterance.attention),
                    _ => None,
                })
                .collect();
            assert_eq!(attended.len(), 1);
            if let Some(attended) = attended[0].first()
                && named.last() != Some(attended)
            {
                named.push(attended.clone());
            }
        }
        assert_eq!(
            named,
            [
                Resource::panel("panel-2"),
                Resource::panel_row("panel-2", 0, "pod7"),
                Resource::panel_row("panel-2", 1, "pod112"),
                Resource::panel_row("panel-2", 2, "pod9"),
                Resource::panel("panel-2"),
            ]
        );
    }

    #[test]
    fn a_row_of_a_list_waiting_for_its_layout_is_neither_named_nor_chosen() {
        use tomori_protocol::{Panel, Place, Pose};
        let ahead = arrival::HOME + Vec3::new(0.0, 0.0, -2.0);
        let mut conversation = lobby(ConnectionRole::Collaborator);
        show_queue(&mut conversation, ahead, arrival::HOME);
        let queue = conversation
            .projection()
            .unwrap()
            .panel("panel-2")
            .unwrap()
            .clone();
        // Two other panels, beside it, and the queue, all changed together, the queue last.
        let shown = |version: u64| {
            let other = |index: usize| {
                let at = arrival::HOME + Vec3::new(3.0 + index as f32, 0.0, -2.0);
                Panel::new(
                    format!("other-{index}"),
                    format!("Other {index}"),
                    format!("- at {version}"),
                    Place::Free {
                        pose: Pose::upright_facing(at.to_array(), arrival::HOME.to_array()),
                    },
                )
                .with_version(version)
            };
            tomori_client::Event::Message(tomori_protocol::ServerMessage::Projection {
                projection: Box::new(
                    tomori_protocol::Projection::new(SPACE, "sky")
                        .with_walls(vec![tomori_wall()])
                        .with_panels(vec![
                            other(0),
                            other(1),
                            queue.clone().with_version(version),
                        ]),
                ),
            })
        };
        conversation.apply(shown(6));
        let mut step = Step::default();
        let row = queue_rows(&mut step, &mut conversation, ahead)[1];
        let now = Instant::now();
        step.update(&row, &mut conversation, &mut felt(), now, FRAME);
        assert!(conversation.pointed_row().is_some());
        // Changed, the queue waits its turn behind the others: its row is not pointed at, named or chosen.
        conversation.apply(shown(7));
        step.update(&row, &mut conversation, &mut felt(), now, FRAME);
        assert_eq!(conversation.pointed_item(), None);
        assert_eq!(conversation.pointed_row(), None);
        step.buttons(&a_down(), true, &mut conversation, &mut felt(), now);
        step.buttons(
            &Buttons::default(),
            true,
            &mut conversation,
            &mut felt(),
            now,
        );
        assert!(conversation.take_actions().is_empty());
        // Its turn come, it is chosen as it is now.
        for _ in 0..4 {
            step.update(&row, &mut conversation, &mut felt(), now, FRAME);
        }
        step.buttons(&a_down(), true, &mut conversation, &mut felt(), now);
        assert_eq!(
            conversation.take_actions(),
            [Action::Send(
                tomori_protocol::ClientMessage::ChoosePanelRow {
                    panel: "panel-2".into(),
                    row: "pod112".into(),
                    revision: Some(7),
                }
            )]
        );
    }

    #[test]
    fn a_or_the_trigger_on_a_choosable_row_chooses_it_once_for_its_environment() {
        let ahead = arrival::HOME + Vec3::new(0.0, 0.0, -2.0);
        let mut conversation = lobby(ConnectionRole::Collaborator);
        show_queue(&mut conversation, ahead, arrival::HOME);
        let mut step = Step::default();
        let rows = queue_rows(&mut step, &mut conversation, ahead);
        let [unchoosable, choosable, other] = rows;
        let now = Instant::now();
        let choose = |row: &str| {
            Action::Send(tomori_protocol::ClientMessage::ChoosePanelRow {
                panel: "panel-2".into(),
                row: row.into(),
                revision: Some(6),
            })
        };
        let hint = |step: &mut Step, conversation: &mut Conversation, senses: &Senses| {
            let output = step.update(senses, conversation, &mut felt(), now, FRAME);
            label_hint(&output.projection)
        };
        // On a choosable row, the label says A chooses it; on one that is not, it says nothing of choosing.
        assert_eq!(
            hint(&mut step, &mut conversation, &choosable).as_deref(),
            Some("A  choose")
        );
        // A chooses it once, with the choosing cue, however many frames it is held, or skipped, before it is let go.
        let mut feedback = felt();
        step.buttons(&a_down(), true, &mut conversation, &mut feedback, now);
        assert!(chose(&mut feedback));
        assert_eq!(conversation.take_actions(), [choose("pod112")]);
        for _ in 0..3 {
            step.buttons(&a_held(), true, &mut conversation, &mut felt(), now);
        }
        step.update(&choosable, &mut conversation, &mut felt(), now, FRAME);
        assert!(conversation.take_actions().is_empty());
        // Just chosen, the label says so.
        assert_eq!(
            hint(&mut step, &mut conversation, &choosable).as_deref(),
            Some("✓  chosen")
        );
        step.buttons(
            &Buttons::default(),
            true,
            &mut conversation,
            &mut felt(),
            now,
        );
        // The trigger, pulled on another choosable row, chooses it once pressed there.
        assert_eq!(
            hint(&mut step, &mut conversation, &other).as_deref(),
            Some("A  choose")
        );
        let mut feedback = felt();
        let pulled = Senses {
            trigger: Some(1.0),
            attractor: Some(arrival::HOME),
            ..other
        };
        step.update(&pulled, &mut conversation, &mut feedback, now, FRAME);
        assert!(chose(&mut feedback));
        assert_eq!(conversation.take_actions(), [choose("pod9")]);
        for _ in 0..3 {
            step.update(&pulled, &mut conversation, &mut felt(), now, FRAME);
        }
        assert!(conversation.take_actions().is_empty());
        step.update(&other, &mut conversation, &mut felt(), now, FRAME);
        // On a row that may not be chosen, A does nothing new, nor does the trigger, as anywhere on a panel in a space.
        let shown = hint(&mut step, &mut conversation, &unchoosable);
        assert!(!shown.unwrap_or_default().contains("choose"));
        step.buttons(&a_down(), true, &mut conversation, &mut felt(), now);
        step.buttons(
            &Buttons::default(),
            true,
            &mut conversation,
            &mut felt(),
            now,
        );
        assert!(conversation.take_actions().is_empty());
        let pulled = Senses {
            trigger: Some(1.0),
            attractor: Some(arrival::HOME),
            ..unchoosable
        };
        step.update(&pulled, &mut conversation, &mut felt(), now, FRAME);
        assert!(conversation.take_actions().is_empty());
    }

    #[test]
    fn the_trigger_on_a_choosable_row_decides_no_request_and_a_does_not_choose_while_one_waits() {
        use tomori_protocol::{Approval, ApprovalOption};
        let ahead = arrival::HOME + Vec3::new(0.0, 0.0, -2.0);
        let mut conversation = lobby(ConnectionRole::Collaborator);
        show_queue(&mut conversation, ahead, arrival::HOME);
        conversation.apply(space(|state| {
            state.projection = true;
            state.choices = true;
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
        conversation.settle_request();
        conversation.take_actions();
        let mut step = Step::default();
        let row = queue_rows(&mut step, &mut conversation, ahead)[1];
        let now = Instant::now();
        step.update(&row, &mut conversation, &mut felt(), now, FRAME);
        // The trigger never decides a request, so a pull on the row chooses it, and the request still waits.
        step.update(
            &Senses {
                trigger: Some(1.0),
                ..row
            },
            &mut conversation,
            &mut felt(),
            now,
            FRAME,
        );
        assert_eq!(
            conversation.take_actions(),
            [Action::Send(
                tomori_protocol::ClientMessage::ChoosePanelRow {
                    panel: "panel-2".into(),
                    row: "pod112".into(),
                    revision: Some(6),
                }
            )]
        );
        assert!(conversation.is_choosing());
        step.update(&row, &mut conversation, &mut felt(), now, FRAME);
        // A, on the row, decides the request rather than choose the row.
        step.buttons(&a_down(), true, &mut conversation, &mut felt(), now);
        let actions = conversation.take_actions();
        assert!(matches!(
            actions[..],
            [Action::Send(tomori_protocol::ClientMessage::Decide { .. })]
        ));
    }

    #[test]
    fn a_visitor_s_a_or_trigger_on_a_choosable_row_chooses_nothing() {
        let ahead = arrival::HOME + Vec3::new(0.0, 0.0, -2.0);
        let mut conversation = lobby(ConnectionRole::Visitor);
        show_queue(&mut conversation, ahead, arrival::HOME);
        let mut step = Step::default();
        let rows = queue_rows(&mut step, &mut conversation, ahead);
        let choosable = rows[1];
        let now = Instant::now();
        let output = step.update(&choosable, &mut conversation, &mut felt(), now, FRAME);
        assert!(
            !label_hint(&output.projection)
                .unwrap_or_default()
                .contains("choose")
        );
        let mut feedback = felt();
        step.buttons(&a_down(), true, &mut conversation, &mut feedback, now);
        assert!(!chose(&mut feedback));
        assert!(conversation.a_refused());
        assert!(conversation.take_actions().is_empty());
        step.buttons(
            &Buttons::default(),
            true,
            &mut conversation,
            &mut felt(),
            now,
        );
        let pulled = Senses {
            trigger: Some(1.0),
            attractor: Some(arrival::HOME),
            ..choosable
        };
        step.update(&pulled, &mut conversation, &mut felt(), now, FRAME);
        assert!(conversation.take_actions().is_empty());
    }

    /// Where, scanning down the agenda [`show_checklist`] floats at `ahead`, the ray first meets a row whose label says
    /// `hint`, and the label there.
    fn ray_on_item(
        step: &mut Step,
        conversation: &mut Conversation,
        ahead: Vec3,
        hint: &str,
    ) -> Option<Senses> {
        (0..60).find_map(|step_down| {
            let at = ahead + Vec3::new(-0.05, 0.15 - step_down as f32 * 0.005, 0.0);
            let senses = Senses {
                beam: true,
                ray: Ray::new(arrival::HOME, at - arrival::HOME),
                ..senses()
            };
            let output = step.update(&senses, conversation, &mut felt(), Instant::now(), FRAME);
            (label_hint(&output.projection).as_deref() == Some(hint)).then_some(senses)
        })
    }

    #[test]
    fn a_pointing_at_a_checklist_item_ticks_or_unticks_it_for_everyone_with_a_hint_on_the_label() {
        let ahead = arrival::HOME + Vec3::new(0.0, 0.0, -2.0);
        let mut conversation = lobby(ConnectionRole::Collaborator);
        show_checklist(&mut conversation, ahead, arrival::HOME);
        let mut step = Step::default();
        let mut feedback = felt();
        let now = Instant::now();
        let tick = |line: u32, checked: bool, revision: u64| {
            Action::Send(tomori_protocol::ClientMessage::CheckPanelItem {
                panel: "panel-1".into(),
                line,
                checked,
                revision: Some(revision),
            })
        };
        // Above the list, on the panel's title, the label says nothing of ticking.
        let hint = (0..60).find_map(|step_down| {
            let at = ahead + Vec3::new(-0.05, 0.15 - step_down as f32 * 0.005, 0.0);
            let senses = Senses {
                beam: true,
                ray: Ray::new(arrival::HOME, at - arrival::HOME),
                ..senses()
            };
            let output = step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
            label_hint(&output.projection)
        });
        assert!(!hint.expect("the label").contains("tick"));
        // A elsewhere on the panel does nothing new.
        step.buttons(
            &press(|buttons| buttons.a = true),
            true,
            &mut conversation,
            &mut feedback,
            now,
        );
        assert!(conversation.take_actions().is_empty());
        // On the ticked item, the label says A unticks it, and A does, for everyone, with the choosing cue.
        ray_on_item(&mut step, &mut conversation, ahead, "A  untick").expect("the ticked item");
        earcons(&mut feedback);
        step.buttons(
            &press(|buttons| buttons.a = true),
            true,
            &mut conversation,
            &mut feedback,
            now,
        );
        // It says the revision the server sent, which its own ticks shown at once do not change.
        assert_eq!(conversation.take_actions(), [tick(2, false, 4)]);
        assert_eq!(earcons(&mut feedback), [Earcon::Choose]);
        // Shown at once, so the label on it now says A ticks it, and A does.
        let senses = ray_on_item(&mut step, &mut conversation, ahead, "A  tick").expect("unticked");
        step.buttons(
            &press(|buttons| buttons.a = true),
            true,
            &mut conversation,
            &mut feedback,
            now,
        );
        assert_eq!(conversation.take_actions(), [tick(2, true, 4)]);
        // On the item below, not ticked, A ticks it.
        ray_on_item(&mut step, &mut conversation, ahead, "A  tick").expect("the second item");
        step.buttons(
            &press(|buttons| buttons.a = true),
            true,
            &mut conversation,
            &mut feedback,
            now,
        );
        assert_eq!(conversation.take_actions(), [tick(3, true, 4)]);
        // The right wrist's transcript comes first: while it shows, A is its, and ticks nothing.
        let wrist = Senses {
            right_palm: Some(palm_up(0.0)),
            ..senses
        };
        step.update(&wrist, &mut conversation, &mut feedback, now, FRAME);
        step.update(&wrist, &mut conversation, &mut feedback, now + HOLD, FRAME);
        assert!(step.wrist.state().active());
        step.buttons(
            &press(|buttons| buttons.a = true),
            true,
            &mut conversation,
            &mut feedback,
            now + HOLD,
        );
        assert!(conversation.take_actions().is_empty());
        // Carrying the panel, A ticks nothing on it.
        let mut step = Step::default();
        let mut carrying =
            ray_on_item(&mut step, &mut conversation, ahead, "A  untick").expect("the item ticked");
        carrying.grip = Some(0.0);
        step.update(&carrying, &mut conversation, &mut feedback, now, FRAME);
        carrying.grip = Some(1.0);
        step.update(&carrying, &mut conversation, &mut feedback, now, FRAME);
        assert_eq!(step.projector.held(), Some("panel-1"));
        step.buttons(
            &press(|buttons| buttons.a = true),
            true,
            &mut conversation,
            &mut feedback,
            now,
        );
        assert!(conversation.take_actions().is_empty());
    }

    #[test]
    fn a_visitor_s_label_offers_no_ticking_and_a_ticks_nothing() {
        let ahead = arrival::HOME + Vec3::new(0.0, 0.0, -2.0);
        let mut conversation = lobby(ConnectionRole::Visitor);
        show_checklist(&mut conversation, ahead, arrival::HOME);
        let mut step = Step::default();
        let mut feedback = felt();
        assert!(ray_on_item(&mut step, &mut conversation, ahead, "A  tick").is_none());
        assert!(ray_on_item(&mut step, &mut conversation, ahead, "A  untick").is_none());
        // Pointing down the list, A is refused, without the choosing cue.
        for step_down in 0..60 {
            let at = ahead + Vec3::new(-0.05, 0.15 - step_down as f32 * 0.005, 0.0);
            let senses = Senses {
                beam: true,
                ray: Ray::new(arrival::HOME, at - arrival::HOME),
                ..senses()
            };
            let now = Instant::now();
            step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
            earcons(&mut feedback);
            step.buttons(
                &press(|buttons| buttons.a = true),
                true,
                &mut conversation,
                &mut feedback,
                now,
            );
            assert!(conversation.take_actions().is_empty());
            assert!(!earcons(&mut feedback).contains(&Earcon::Choose));
        }
    }

    #[test]
    fn signs_face_whoever_walks_about_them_and_a_worker_pointed_at_from_aside_is_attended_to() {
        use tomori_protocol::Resource;
        let mut conversation = lobby(ConnectionRole::Owner);
        conversation.apply(space(|state| {
            state.workers = vec![tomori_protocol::Worker {
                id: "ses_a".into(),
                parent: None,
                title: "Review".into(),
                state: tomori_protocol::WorkerState::Running,
            }];
        }));
        conversation.take_actions();
        let mut step = Step::default();
        let mut feedback = felt();
        let now = Instant::now();
        let mut senses = Senses {
            beam: true,
            ..senses()
        };
        for _ in 0..200 {
            step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
        }
        let output = step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
        let worker = Vec3::from_array(output.stations[0].position);
        // Signs face this participant, not whoever the agent faces.
        assert_eq!(output.sign_facing, Some(arrival::HOME));
        // From in front, 45° to its left and 70° to its right, about the worker, as far from it as home is: the signs
        // turn to face them there, and the ray from there at the worker names it as the words begin.
        let from_home = (arrival::HOME - worker).with_y(0.0);
        for degrees in [0.0_f32, 45.0, -70.0] {
            let eyes = worker
                + glam::Quat::from_rotation_y(degrees.to_radians()) * from_home
                + Vec3::Y * (arrival::HOME.y - worker.y);
            let head = Mat4::from_translation(eyes);
            senses.head = Some(head);
            senses.head_pose = Pose::from_transform(head);
            senses.viewer = Some(eyes);
            senses.ray = Ray::new(eyes, worker - eyes);
            let mut facing = None;
            for _ in 0..72 {
                facing = step
                    .update(&senses, &mut conversation, &mut feedback, now, FRAME)
                    .sign_facing;
            }
            let facing = facing.expect("signs face the viewer");
            assert!(
                facing.distance(eyes)
                    <= crate::sign_facing::DEAD_ZONE + crate::sign_facing::SETTLE + 1e-4,
                "{degrees}: {facing} {eyes}"
            );
            conversation.take_actions();
            conversation.talk(true);
            conversation.talk(false);
            let actions = conversation.take_actions();
            let Some(Action::BeginUtterance(utterance)) = actions.first() else {
                unreachable!("recording begins, not {actions:?}")
            };
            assert_eq!(
                utterance.attention,
                [Resource::worker("ses_a")],
                "{degrees}"
            );
        }
        // Without a viewer, as a spectator's, they face whoever the agent faces.
        senses.viewer = None;
        let output = step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
        assert_eq!(output.sign_facing, None);
    }

    #[test]
    fn the_ray_names_a_panel_and_the_grip_pulls_it_off_unless_a_card_is_nearer() {
        use tomori_protocol::{
            Card, CardKind, Cards, Checks, Panel, Place, Pose, Resource, ServerMessage,
        };
        let mut conversation = lobby(ConnectionRole::Owner);
        conversation.apply(space(|state| {
            state.cards = true;
            state.projection = true;
        }));
        let facing_home = |at: Vec3| Pose::upright_facing(at.to_array(), arrival::HOME.to_array());
        let ahead = arrival::HOME + Vec3::new(0.0, 0.0, -3.0);
        let project = |conversation: &mut Conversation| {
            conversation.apply(tomori_client::Event::Message(ServerMessage::Projection {
                projection: Box::new(
                    tomori_protocol::Projection::new(SPACE, "sky")
                        .with_walls(vec![tomori_wall()])
                        .with_panels(vec![Panel::new(
                            "panel-1",
                            "Build status",
                            "Green.",
                            Place::Free {
                                pose: facing_home(ahead),
                            },
                        )]),
                ),
            }));
            conversation.take_actions();
        };
        project(&mut conversation);
        let mut step = Step::default();
        let now = Instant::now();
        let mut senses = Senses {
            beam: true,
            ray: Ray::new(arrival::HOME, -Vec3::Z),
            ..senses()
        };
        let output = step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        // The Tomori wall's six slots and its label, the panel, and the label on the ray naming it.
        let wall = tomori_protocol::WALL_SLOTS as usize + 1;
        assert_eq!(output.projection.len(), wall + 2);
        let label = output.projection[wall + 1]
            .canvas
            .as_deref()
            .unwrap()
            .plain();
        assert!(label.starts_with("Build status"), "{label}");
        let beam = output
            .arrival
            .as_ref()
            .and_then(|arrival| arrival.ray)
            .unwrap();
        assert!(beam.hit && beam.end.distance(ahead) < 1e-3);
        let attending = |conversation: &mut Conversation| {
            conversation.talk(true);
            conversation.talk(false);
            let actions = conversation.take_actions();
            let Some(Action::BeginUtterance(utterance)) = actions.first() else {
                unreachable!("recording begins, not {actions:?}")
            };
            utterance.attention.clone()
        };
        assert_eq!(attending(&mut conversation), [Resource::panel("panel-1")]);
        // A visitor's squeeze pulls nothing off.
        let role = |role| tomori_client::Event::Message(ServerMessage::Role { role });
        conversation.apply(role(ConnectionRole::Visitor));
        senses.grip = Some(1.0);
        step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        assert_eq!(step.projector.held(), None);
        senses.grip = Some(0.0);
        step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        conversation.apply(role(ConnectionRole::Owner));
        conversation.take_actions();
        // Squeezed, it is pulled off and carried; let go, it floats where it was left, for everyone.
        senses.grip = Some(1.0);
        step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        assert_eq!(step.projector.held(), Some("panel-1"));
        senses.ray = Ray::new(arrival::HOME, Vec3::new(-0.5, 0.0, -1.0));
        let output = step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        // No label on a panel carried, but the hint beside it, last, over everything else.
        assert_eq!(output.projection.len(), wall + 2);
        let hint = output.projection[wall + 1]
            .canvas
            .as_deref()
            .unwrap()
            .plain();
        assert!(hint.contains("throw away, for all"), "{hint}");
        senses.grip = Some(0.0);
        step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        assert!(matches!(
            conversation.take_actions().as_slice(),
            [Action::Send(tomori_protocol::ClientMessage::PlacePanel { panel, wall: None, slot: None, pose: Some(_) })] if panel == "panel-1"
        ));
        // A card nearer on the ray is pointed at and picked up instead.
        project(&mut conversation);
        conversation.apply(tomori_client::Event::Message(ServerMessage::Cards {
            cards: Box::new(Cards {
                space: SPACE.into(),
                source: "o/r".into(),
                cards: vec![Card {
                    id: "github://o/r/issues/1".into(),
                    number: 1,
                    kind: CardKind::Issue,
                    title: "A card".into(),
                    author: "ada".into(),
                    labels: Vec::new(),
                    checks: Checks::None,
                    age: 0,
                    placement: Some(facing_home(arrival::HOME - Vec3::Z * 1.5)),
                }],
                status: None,
            }),
        }));
        let mut step = Step::default();
        senses.ray = Ray::new(arrival::HOME, -Vec3::Z);
        step.update(
            &senses,
            &mut conversation,
            &mut felt(),
            now + crate::projection::PENDING,
            FRAME,
        );
        assert_eq!(
            attending(&mut conversation),
            [Resource::card("github://o/r/issues/1")]
        );
        senses.grip = Some(1.0);
        step.update(
            &senses,
            &mut conversation,
            &mut felt(),
            now + crate::projection::PENDING,
            FRAME,
        );
        assert_eq!(step.board.held(), Some("github://o/r/issues/1"));
        assert_eq!(step.projector.held(), None);
    }

    #[test]
    fn the_ray_points_at_the_nearer_of_a_card_and_a_worker_and_the_grip_carries_a_card() {
        use tomori_protocol::{Card, CardKind, Cards, Checks, Resource, ServerMessage};
        let mut conversation = lobby(ConnectionRole::Owner);
        conversation.apply(space(|state| {
            state.cards = true;
            state.workers = vec![tomori_protocol::Worker {
                id: "ses_a".into(),
                parent: None,
                title: "Review".into(),
                state: tomori_protocol::WorkerState::Running,
            }];
        }));
        let mut step = Step::default();
        let mut feedback = felt();
        let now = Instant::now();
        let mut senses = Senses {
            beam: true,
            ..senses()
        };
        let mut frame = |step: &mut Step, conversation: &mut Conversation, senses: &Senses| {
            step.update(senses, conversation, &mut feedback, now, FRAME)
                .stations
                .iter()
                .map(|station| Vec3::from_array(station.position))
                .collect::<Vec<_>>()
        };
        let worker = frame(&mut step, &mut conversation, &senses)[0];
        // Before the cards arrive, there are none to draw.
        assert!(
            step.update(&senses, &mut conversation, &mut felt(), now, FRAME)
                .cards
                .is_empty()
        );
        // One card stands halfway to the worker, facing the eye, and another beyond it.
        let between = arrival::HOME.lerp(worker, 0.5);
        let beyond = arrival::HOME.lerp(worker, 1.5);
        let placed = |at: Vec3| {
            let model = crate::recording::facing(at, arrival::HOME, 1.0);
            let (_, rotation, _) = model.to_scale_rotation_translation();
            tomori_protocol::Pose {
                position: at.to_array(),
                orientation: rotation.to_array(),
            }
        };
        let card = |number: u64, at: Vec3| Card {
            id: format!("github://o/r/issues/{number}"),
            number,
            kind: CardKind::Issue,
            title: "A card".into(),
            author: "ada".into(),
            labels: Vec::new(),
            checks: Checks::None,
            age: 0,
            placement: Some(placed(at)),
        };
        let show = |conversation: &mut Conversation, at: Vec3| {
            conversation.apply(tomori_client::Event::Message(ServerMessage::Cards {
                cards: Box::new(Cards {
                    space: SPACE.into(),
                    source: "o/r".into(),
                    cards: vec![card(1, at)],
                    status: None,
                }),
            }));
        };
        show(&mut conversation, between);
        conversation.take_actions();
        senses.ray = Ray::new(arrival::HOME, worker - arrival::HOME);
        let output = step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        assert_eq!(output.cards.len(), 2, "the inbox and the card");
        let beam = output
            .arrival
            .as_ref()
            .and_then(|arrival| arrival.ray)
            .unwrap();
        assert!(beam.hit && beam.end.distance(between) < 1e-3);
        // What X begins attending to as it is pressed.
        let attending = |conversation: &mut Conversation| {
            conversation.talk(true);
            conversation.talk(false);
            let actions = conversation.take_actions();
            let Some(Action::BeginUtterance(utterance)) = actions.first() else {
                unreachable!("recording begins, not {actions:?}")
            };
            utterance.attention.clone()
        };
        let card_pointed = Resource::card("github://o/r/issues/1");
        assert_eq!(
            attending(&mut conversation),
            std::slice::from_ref(&card_pointed)
        );
        // Beyond the worker, the worker is pointed at.
        show(&mut conversation, beyond);
        frame(&mut step, &mut conversation, &senses);
        assert_eq!(attending(&mut conversation), [Resource::worker("ses_a")]);
        // And the ray ends on the worker, the nearer.
        let output = step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        let beam = output
            .arrival
            .as_ref()
            .and_then(|arrival| arrival.ray)
            .unwrap();
        let met = step.delegation_scene.hit_point().unwrap();
        assert!(beam.hit && beam.end.distance(met) < 1e-3);
        assert!(beam.end.distance(arrival::HOME) < beyond.distance(arrival::HOME));
        // Back between, the grip picks it up and, carried, it is what is pointed at, wherever the ray goes.
        show(&mut conversation, between);
        frame(&mut step, &mut conversation, &senses);
        senses.grip = Some(1.0);
        frame(&mut step, &mut conversation, &senses);
        assert_eq!(step.board.held(), Some("github://o/r/issues/1"));
        // In an environment that allows travel, the left stick does not aim while a card is carried, which would put it down.
        conversation.set_locomotion(vec![(
            crate::test_support::SKY,
            crate::locomotion::Locomotion::teleport(vec![crate::locomotion::Surface::disc(
                Vec3::ZERO,
                20.0,
            )]),
        )]);
        // Once the scenery has grown in, where travel could otherwise aim.
        for _ in 0..400 {
            step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        }
        assert_eq!(step.board.held(), Some("github://o/r/issues/1"));
        conversation.take_actions();
        step.buttons(
            &press(|buttons| buttons.left_stick = Vec2::Y),
            true,
            &mut conversation,
            &mut felt(),
            now,
        );
        let output = step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        assert_eq!(output.landing, None);
        assert_eq!(step.board.held(), Some("github://o/r/issues/1"));
        assert!(conversation.take_actions().is_empty(), "nothing put down");
        step.buttons(
            &Buttons::default(),
            true,
            &mut conversation,
            &mut felt(),
            now,
        );
        conversation.set_locomotion(Vec::new());
        senses.ray = Ray::new(arrival::HOME, Vec3::new(0.5, 0.0, -1.0));
        let output = step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        let beam = output
            .arrival
            .as_ref()
            .and_then(|arrival| arrival.ray)
            .unwrap();
        assert!(
            beam.end
                .distance(senses.ray.unwrap().at(between.distance(arrival::HOME)))
                < 1e-3
        );
        assert_eq!(attending(&mut conversation), [card_pointed]);
        // Letting go puts it down there, for everyone in the space.
        senses.grip = Some(0.0);
        frame(&mut step, &mut conversation, &senses);
        let actions = conversation.take_actions();
        let [Action::Send(tomori_protocol::ClientMessage::PlaceCard { card, placement })] =
            actions.as_slice()
        else {
            unreachable!("one card put down, not {actions:?}");
        };
        assert_eq!(card, "github://o/r/issues/1");
        assert!(placement.is_some());
    }

    #[test]
    fn labels_face_the_viewer_in_steps_of_five_centimetres() {
        let mut conversation = joined();
        let mut step = Step::default();
        let start = Instant::now();
        let swaying = Senses {
            left_palm: Some(left_palm_up(0.0)),
            viewer: Some(arrival::HOME + Vec3::new(0.012, -0.013, 0.031)),
            ..senses()
        };
        step.update(&swaying, &mut conversation, &mut felt(), start, FRAME);
        let frame = step.update(
            &swaying,
            &mut conversation,
            &mut felt(),
            start + HOLD,
            FRAME,
        );
        let viewer = frame.arrival.expect("the menu").scene.viewer.unwrap();
        assert!(
            viewer.abs_diff_eq(Vec3::new(0.0, 1.6, 0.05), 1e-6),
            "{viewer}"
        );
        // The desktop's viewer, at home, is already on the grid, so it is unchanged.
        assert_eq!((arrival::HOME * 20.0).round() / 20.0, arrival::HOME);
    }

    #[test]
    fn a_stick_push_navigates_once_until_it_returns_and_never_in_a_space() {
        let mut conversation = Conversation::new("Ada");
        conversation.set_servers(
            ["10.0.0.1:8765", "10.0.0.2:8765", "10.0.0.3:8765"]
                .map(|address| {
                    tomori_client::server_choice::Choice::new(
                        address,
                        address,
                        tomori_client::server_choice::Source::Discovered,
                    )
                })
                .to_vec(),
        );
        let mut feedback = felt();
        let mut step = Step::default();
        let now = Instant::now();
        let down = press(|buttons| buttons.left_stick = Vec2::new(0.0, -1.0));
        let before = conversation.text();
        step.buttons(&down, true, &mut conversation, &mut feedback, now);
        let after = conversation.text();
        assert_ne!(before, after, "a push moves");
        step.buttons(&down, true, &mut conversation, &mut feedback, now);
        assert_eq!(conversation.text(), after, "holding it does not repeat");
        step.buttons(
            &Buttons::default(),
            true,
            &mut conversation,
            &mut feedback,
            now,
        );
        step.buttons(
            &press(|buttons| buttons.left_stick = Vec2::new(0.0, 1.0)),
            true,
            &mut conversation,
            &mut feedback,
            now,
        );
        assert_eq!(conversation.text(), before, "pushing back moves back");
        // Sideways, it moves between pages rather than rows, and all three servers are on one page.
        step.buttons(
            &Buttons::default(),
            true,
            &mut conversation,
            &mut feedback,
            now,
        );
        for x in [1.0, -1.0] {
            step.buttons(
                &press(|buttons| buttons.left_stick = Vec2::new(x, 0.2)),
                true,
                &mut conversation,
                &mut feedback,
                now,
            );
            step.buttons(
                &Buttons::default(),
                true,
                &mut conversation,
                &mut feedback,
                now,
            );
        }
        assert_eq!(conversation.text(), before);
        let mut conversation = joined();
        let before = conversation.text();
        step.buttons(&down, true, &mut conversation, &mut feedback, now);
        assert_eq!(conversation.text(), before);
    }

    #[test]
    fn holding_b_in_a_space_ramps_then_leaves_once_the_hold_is_long_enough() {
        let mut conversation = joined();
        let mut feedback = felt();
        let mut step = Step::default();
        let start = Instant::now();
        let held = press(|buttons| buttons.back_held = true);
        step.buttons(
            &press(|buttons| {
                buttons.back = true;
                buttons.back_held = true;
            }),
            true,
            &mut conversation,
            &mut feedback,
            start,
        );
        step.buttons(
            &held,
            true,
            &mut conversation,
            &mut feedback,
            start + LEAVE_HOLD / 2,
        );
        // The right controller ramps from light to firm as it is held.
        let ramp = felt_on(&mut feedback, Side::Right, start + LEAVE_HOLD / 2);
        assert!(
            matches!(ramp[..], [Haptic::Vibrate { amplitude, .. }] if (amplitude - 0.35).abs() < 0.01),
            "{ramp:?}"
        );
        assert!(conversation.joined_space().is_some());
        step.buttons(
            &held,
            true,
            &mut conversation,
            &mut feedback,
            start + LEAVE_HOLD,
        );
        assert!(earcons(&mut feedback).contains(&Earcon::Leave));
        assert!(
            conversation.take_actions().iter().any(|action| matches!(
                action,
                Action::Send(tomori_protocol::ClientMessage::Leave)
            ))
        );
    }

    #[test]
    fn letting_go_of_b_early_stays_and_stops_the_ramp() {
        let mut conversation = joined();
        let mut feedback = felt();
        let mut step = Step::default();
        let start = Instant::now();
        step.buttons(
            &press(|buttons| {
                buttons.back = true;
                buttons.back_held = true;
            }),
            true,
            &mut conversation,
            &mut feedback,
            start,
        );
        let half = start + LEAVE_HOLD / 2;
        step.buttons(
            &press(|buttons| buttons.back_held = true),
            true,
            &mut conversation,
            &mut feedback,
            half,
        );
        assert!(!felt_on(&mut feedback, Side::Right, half).is_empty());
        step.buttons(
            &Buttons::default(),
            true,
            &mut conversation,
            &mut feedback,
            half,
        );
        assert_eq!(felt_on(&mut feedback, Side::Right, half), [Haptic::Stop]);
        step.buttons(
            &press(|buttons| buttons.back_held = true),
            true,
            &mut conversation,
            &mut feedback,
            start + LEAVE_HOLD * 2,
        );
        assert!(conversation.joined_space().is_some());
    }

    #[test]
    fn b_held_from_before_joining_never_leaves() {
        let mut conversation = lobby(ConnectionRole::Owner);
        let mut feedback = felt();
        let mut step = Step::default();
        let start = Instant::now();
        step.buttons(
            &press(|buttons| {
                buttons.back = true;
                buttons.back_held = true;
            }),
            true,
            &mut conversation,
            &mut feedback,
            start,
        );
        assert!(step.back_since.is_none());
        // Held on into a space, it never leaves it.
        let mut conversation = joined();
        for seconds in 1..4 {
            step.buttons(
                &press(|buttons| buttons.back_held = true),
                true,
                &mut conversation,
                &mut feedback,
                start + Duration::from_secs(seconds),
            );
        }
        assert!(conversation.joined_space().is_some());
    }

    #[test]
    fn x_talks_while_held_and_after_focus_loss_only_once_released() {
        let mut conversation = joined();
        let mut feedback = felt();
        let mut step = Step::default();
        let now = Instant::now();
        let talk = press(|buttons| buttons.talk = true);
        step.buttons(&talk, true, &mut conversation, &mut feedback, now);
        assert!(conversation.recording().is_some());
        step.blur(&mut conversation);
        assert!(conversation.recording().is_none());
        // Still held through the system menu, it does not record again.
        step.buttons(&talk, true, &mut conversation, &mut feedback, now);
        assert!(conversation.recording().is_none());
        step.buttons(
            &Buttons::default(),
            true,
            &mut conversation,
            &mut feedback,
            now,
        );
        step.buttons(&talk, true, &mut conversation, &mut feedback, now);
        assert!(conversation.recording().is_some());
        // Without focus, X does nothing.
        step.buttons(
            &Buttons::default(),
            true,
            &mut conversation,
            &mut feedback,
            now,
        );
        let mut unfocused = Step::default();
        let mut conversation = joined();
        unfocused.buttons(&talk, false, &mut conversation, &mut feedback, now);
        assert!(conversation.recording().is_none());
    }

    #[test]
    fn y_pins_the_menu_in_a_space_until_b_and_lists_again_elsewhere() {
        let mut conversation = joined();
        let mut feedback = felt();
        let mut step = Step::default();
        let now = Instant::now();
        let y = press(|buttons| buttons.y = true);
        step.buttons(&y, true, &mut conversation, &mut feedback, now);
        assert!(step.pinned());
        assert!(
            !felt_on(&mut feedback, Side::Left, now).is_empty(),
            "pinning is felt"
        );
        // B lets go of it before anything else, so the space is not backed out of or left.
        step.buttons(
            &press(|buttons| {
                buttons.back = true;
                buttons.back_held = true;
            }),
            true,
            &mut conversation,
            &mut feedback,
            now,
        );
        assert!(!step.pinned());
        step.buttons(&y, true, &mut conversation, &mut feedback, now);
        step.buttons(&y, true, &mut conversation, &mut feedback, now);
        assert!(!step.pinned(), "Y again lets go");
        assert!(felt_on(&mut feedback, Side::Left, now).len() == 1);
        // Leaving the space lets go of a pinned menu.
        step.buttons(&y, true, &mut conversation, &mut feedback, now);
        let mut lobby = lobby(ConnectionRole::Owner);
        step.buttons(&Buttons::default(), true, &mut lobby, &mut feedback, now);
        assert!(!step.pinned());
        // Outside a space, Y lists the sessions again.
        step.buttons(&y, true, &mut lobby, &mut feedback, now);
        assert!(!step.pinned());
        assert!(!lobby.take_actions().is_empty());
    }

    #[test]
    fn mute_and_replay_reach_the_conversation() {
        let mut conversation = joined();
        let mut feedback = felt();
        let mut step = Step::default();
        let now = Instant::now();
        let muted = |conversation: &Conversation| conversation.menu_model().muted;
        let before = muted(&conversation);
        step.buttons(
            &press(|buttons| buttons.mute = true),
            true,
            &mut conversation,
            &mut feedback,
            now,
        );
        assert_ne!(muted(&conversation), before);
        step.buttons(
            &press(|buttons| buttons.replay = true),
            true,
            &mut conversation,
            &mut feedback,
            now,
        );
        // This server does not say what it speaks, so it cannot play a reply again, and says so.
        assert_eq!(
            conversation.menu_model().notice.as_deref(),
            Some("This server cannot play replies again.")
        );
    }

    #[test]
    fn the_left_palm_within_45_degrees_of_up_shows_the_menu_after_the_dwell_until_past_55() {
        let mut conversation = joined();
        let mut feedback = felt();
        let start = Instant::now();
        let at = |degrees| Senses {
            left_palm: Some(left_palm_up(degrees)),
            ..senses()
        };
        let mut step = Step::default();
        let frame = step.update(&at(40.0), &mut conversation, &mut feedback, start, FRAME);
        assert!(frame.arrival.is_none(), "not before the dwell");
        let frame = step.update(
            &at(40.0),
            &mut conversation,
            &mut feedback,
            start + HOLD / 2,
            FRAME,
        );
        assert!(frame.arrival.is_none(), "not halfway through it");
        let frame = step.update(
            &at(40.0),
            &mut conversation,
            &mut feedback,
            start + HOLD,
            FRAME,
        );
        let arrival = frame.arrival.expect("the menu");
        assert!(!arrival.scene.objects.is_empty());
        assert_eq!(arrival.scene.viewer, Some(arrival::HOME));
        // The pill hides while a hand's panel shows.
        assert_eq!(frame.panels.pill, Sheet::default());
        // Between the angles it stays, and past 55° it hides.
        let frame = step.update(
            &at(50.0),
            &mut conversation,
            &mut feedback,
            start + HOLD * 2,
            FRAME,
        );
        assert!(frame.arrival.is_some());
        let frame = step.update(
            &at(60.0),
            &mut conversation,
            &mut feedback,
            start + HOLD * 3,
            FRAME,
        );
        assert!(frame.arrival.is_none());
        // 50° from hidden never shows it.
        let mut step = Step::default();
        step.update(&at(50.0), &mut conversation, &mut feedback, start, FRAME);
        let frame = step.update(
            &at(50.0),
            &mut conversation,
            &mut feedback,
            start + HOLD,
            FRAME,
        );
        assert!(frame.arrival.is_none());
        // Without focus it never shows.
        let unfocused = Senses {
            focused: false,
            ..at(0.0)
        };
        let mut step = Step::default();
        step.update(&unfocused, &mut conversation, &mut feedback, start, FRAME);
        let frame = step.update(
            &unfocused,
            &mut conversation,
            &mut feedback,
            start + HOLD,
            FRAME,
        );
        assert!(frame.arrival.is_none());
    }

    #[test]
    fn turning_the_head_does_not_change_whether_either_palm_shows_its_panel() {
        let mut conversation = joined();
        let start = Instant::now();
        let neck = arrival::HOME - Vec3::new(0.0, 0.075, -0.08);
        for yaw in [-90.0, -45.0, 0.0, 45.0, 90.0, 180.0] {
            let head = crate::hand_menu::head_turned(neck, yaw, -20.0);
            let turned = Senses {
                head: Some(head),
                head_pose: Pose::from_transform(head),
                viewer: Some(crate::hand_menu::head_centre(head)),
                ..senses()
            };
            for (degrees, shown) in [(0.0, true), (40.0, true), (-40.0, true), (90.0, false)] {
                let mut step = Step::default();
                let left = Senses {
                    left_palm: Some(left_palm_up(degrees)),
                    ..turned
                };
                step.update(&left, &mut conversation, &mut felt(), start, FRAME);
                let frame = step.update(&left, &mut conversation, &mut felt(), start + HOLD, FRAME);
                assert_eq!(frame.arrival.is_some(), shown, "left {yaw} {degrees}");
                let mut step = Step::default();
                let right = Senses {
                    right_palm: Some(palm_up(degrees)),
                    ..turned
                };
                let wrist = wrist_after(&mut step, &mut conversation, &right, start);
                assert_eq!(wrist.model.is_some(), shown, "right {yaw} {degrees}");
            }
        }
    }

    /// The transcript's sheet after holding the right palm `degrees` from up, from hidden.
    fn wrist_after(
        step: &mut Step,
        conversation: &mut Conversation,
        senses: &Senses,
        start: Instant,
    ) -> Sheet {
        step.update(senses, conversation, &mut felt(), start, FRAME);
        step.update(senses, conversation, &mut felt(), start + HOLD, FRAME)
            .panels
            .wrist
    }

    #[test]
    fn the_right_palm_within_45_degrees_of_up_shows_the_transcript_after_the_dwell_until_past_55() {
        let mut conversation = joined();
        let start = Instant::now();
        let at = |degrees| Senses {
            right_palm: Some(palm_up(degrees)),
            ..senses()
        };
        let mut step = Step::default();
        let mut feedback = felt();
        let frame = step.update(&at(40.0), &mut conversation, &mut feedback, start, FRAME);
        assert!(frame.panels.wrist.model.is_none(), "not before the dwell");
        let frame = step.update(
            &at(40.0),
            &mut conversation,
            &mut feedback,
            start + HOLD / 2,
            FRAME,
        );
        assert!(frame.panels.wrist.model.is_none(), "not halfway through it");
        let frame = step.update(
            &at(40.0),
            &mut conversation,
            &mut feedback,
            start + HOLD,
            FRAME,
        );
        let shown = frame.panels.wrist.clone();
        assert!(shown.model.is_some() && shown.canvas.is_some());
        // Opening is felt lightly on the right hand.
        assert!(!felt_on(&mut feedback, Side::Right, start + HOLD).is_empty());
        // Between the angles it stays.
        let frame = step.update(
            &at(50.0),
            &mut conversation,
            &mut feedback,
            start + HOLD * 2,
            FRAME,
        );
        assert!(frame.panels.wrist.model.is_some());
        // Past 55° it hides.
        let frame = step.update(
            &at(60.0),
            &mut conversation,
            &mut feedback,
            start + HOLD * 3,
            FRAME,
        );
        assert_eq!(frame.panels.wrist, Sheet::default());
        // 50° from hidden never shows it.
        let mut step = Step::default();
        assert!(
            wrist_after(&mut step, &mut conversation, &at(50.0), start)
                .model
                .is_none()
        );
        // Nor does pointing, nor a palm without focus.
        let mut step = Step::default();
        let pointing = Senses {
            pointing: true,
            ..at(0.0)
        };
        assert!(
            wrist_after(&mut step, &mut conversation, &pointing, start)
                .model
                .is_none()
        );
        let mut step = Step::default();
        let unfocused = Senses {
            focused: false,
            ..at(0.0)
        };
        assert!(
            wrist_after(&mut step, &mut conversation, &unfocused, start)
                .model
                .is_none()
        );
        // Outside a space there is no transcript.
        let mut lobby = lobby(ConnectionRole::Owner);
        let mut step = Step::default();
        assert!(
            wrist_after(&mut step, &mut lobby, &at(0.0), start)
                .model
                .is_none()
        );
    }

    #[test]
    fn whichever_of_the_menu_and_the_transcript_shows_first_wins() {
        let mut conversation = joined();
        let start = Instant::now();
        let both = Senses {
            left_palm: Some(left_palm_up(0.0)),
            right_palm: Some(palm_up(0.0)),
            ..senses()
        };
        // The transcript first: the menu then does not show.
        let mut step = Step::default();
        let transcript = Senses {
            left_palm: None,
            ..both
        };
        wrist_after(&mut step, &mut conversation, &transcript, start);
        step.update(
            &both,
            &mut conversation,
            &mut felt(),
            start + HOLD * 2,
            FRAME,
        );
        let frame = step.update(
            &both,
            &mut conversation,
            &mut felt(),
            start + HOLD * 3,
            FRAME,
        );
        assert!(frame.panels.wrist.model.is_some());
        assert!(frame.arrival.is_none(), "no menu");
        // The menu first: the transcript then does not show.
        let mut step = Step::default();
        let menu = Senses {
            right_palm: None,
            ..both
        };
        step.update(&menu, &mut conversation, &mut felt(), start, FRAME);
        step.update(&menu, &mut conversation, &mut felt(), start + HOLD, FRAME);
        step.update(
            &both,
            &mut conversation,
            &mut felt(),
            start + HOLD * 2,
            FRAME,
        );
        let frame = step.update(
            &both,
            &mut conversation,
            &mut felt(),
            start + HOLD * 3,
            FRAME,
        );
        assert!(frame.panels.wrist.model.is_none());
        assert!(frame.arrival.is_some());
    }

    #[test]
    fn the_attractor_is_drawn_as_each_platform_draws_it() {
        // Outside a space, where the trigger still draws the swarm.
        let mut conversation = lobby(ConnectionRole::Owner);
        let mut feedback = felt();
        let now = Instant::now();
        let at = Vec3::new(0.2, 1.0, -0.5);
        for (marker, idle, pulling) in [
            (Marker::Always, Some(at), Some(at)),
            (Marker::WhileAttracting, None, Some(at)),
            (Marker::Hidden, None, None),
        ] {
            let mut step = Step::default();
            let mut senses = Senses {
                attractor: Some(at),
                marker,
                ..senses()
            };
            let frame = step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
            assert!(!frame.attracting);
            assert_eq!(frame.marker, idle, "{marker:?}");
            senses.trigger = Some(1.0);
            let frame = step.update(&senses, &mut conversation, &mut feedback, now, FRAME);
            assert!(frame.attracting, "a pull on nothing attracts");
            assert_eq!(frame.marker, pulling, "{marker:?}");
        }
    }

    /// The [`ClientMessage::PlaceAgent`]s `conversation` sent since it was last asked.
    fn placed(conversation: &mut Conversation) -> Vec<Option<tomori_protocol::Pose>> {
        conversation
            .take_actions()
            .into_iter()
            .filter_map(|action| match action {
                Action::Send(tomori_protocol::ClientMessage::PlaceAgent { at }) => Some(at),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn in_a_space_the_trigger_draws_the_swarm_nowhere_and_summons_nothing_from_any_server() {
        let now = Instant::now();
        let at = Vec3::new(0.8, 1.2, -0.6);
        let pulling = Senses {
            attractor: Some(at),
            trigger: Some(1.0),
            ..senses()
        };
        // On a server that keeps summons, as an older one does, and on one that says how the agent moves alike.
        for older in [true, false] {
            let mut conversation = lobby(ConnectionRole::Owner);
            conversation.apply(space(|state| {
                state.summons = older;
                state.movement = (!older).then(tomori_protocol::AgentMovement::default);
            }));
            conversation.take_actions();
            let mut step = Step::default();
            for _ in 0..(1.0 / FRAME) as usize {
                let frame = step.update(&pulling, &mut conversation, &mut felt(), now, FRAME);
                assert!(!frame.attracting, "{older}");
            }
            step.update(&senses(), &mut conversation, &mut felt(), now, FRAME);
            assert!(placed(&mut conversation).is_empty());
            assert_eq!(
                conversation.arrangement().agent,
                crate::anchors::Anchor::AGENT
            );
            // Away, it is sent home from the hand menu instead, from either.
            conversation.apply(space(|state| {
                state.summons = older;
                state.movement = (!older).then(tomori_protocol::AgentMovement::default);
                state.summoned = Some(tomori_protocol::Pose::at([1.0, 1.4, -1.0]));
            }));
            conversation.send_agent_home();
            assert_eq!(placed(&mut conversation), [None]);
        }
    }

    #[test]
    fn on_an_environment_s_selectable_object_the_trigger_selects_it_and_on_a_panel_does_nothing() {
        let now = Instant::now();
        let at = Vec3::new(0.8, 1.2, -0.6);
        let pull = |step: &mut Step, conversation: &mut Conversation, senses: &Senses| {
            let pulling = Senses {
                attractor: Some(at),
                trigger: Some(1.0),
                ..*senses
            };
            let mut drawn = false;
            for _ in 0..(1.0 / FRAME).ceil() as usize {
                drawn |= step
                    .update(&pulling, conversation, &mut felt(), now, FRAME)
                    .attracting;
            }
            let released = Senses {
                trigger: Some(0.0),
                ..*senses
            };
            step.update(&released, conversation, &mut felt(), now, FRAME);
            drawn
        };
        let (mut conversation, done) = lamp(Default::default(), true);
        conversation.take_actions();
        let mut step = Step::default();
        let on_lamp = Senses {
            ray: Ray::new(arrival::HOME, Vec3::NEG_Z),
            ..senses()
        };
        grow(&mut step, &mut conversation, &on_lamp, now);
        assert_eq!(step.aim, precedence::Aim::Environment);
        assert!(!pull(&mut step, &mut conversation, &on_lamp));
        assert!(!selects(&done).is_empty());
        // On a panel in front of it, Tomori's, the pull draws nothing and summons nothing.
        code_ahead(&mut conversation, 1);
        conversation.apply(space(|state| state.projection = true));
        conversation.take_actions();
        grow(&mut step, &mut conversation, &on_lamp, now);
        assert_eq!(step.aim, precedence::Aim::Tomori);
        assert!(!pull(&mut step, &mut conversation, &on_lamp));
        assert!(placed(&mut conversation).is_empty());
    }

    #[test]
    fn the_agent_glides_where_it_goes_by_its_waypoint_turning_to_face_whoever_it_turns_to() {
        let now = Instant::now();
        let mut conversation = lobby(ConnectionRole::Owner);
        conversation.apply(space(|state| {
            state.movement = Some(tomori_protocol::AgentMovement::default());
        }));
        let mut step = Step::default();
        for _ in 0..10 {
            step.update(&senses(), &mut conversation, &mut felt(), now, FRAME);
        }
        // It goes 2 m to the side, over someone's head by its waypoint, and settles there, never jumping.
        let to = tomori_protocol::Pose::at([2.0, 1.4, -2.3]).facing_towards([0.0, 1.6, 0.0]);
        conversation.apply(space(|state| {
            state.movement = Some(tomori_protocol::AgentMovement::default());
            state.summoned = Some(to);
            state.via = Some([1.0, 2.6, -2.3]);
        }));
        let mut last = crate::swarm::CENTER;
        let mut highest = 0.0f32;
        for _ in 0..(8.0 / FRAME) as usize {
            let frame = step.update(&senses(), &mut conversation, &mut felt(), now, FRAME);
            let at = frame.swarm.anchor().position;
            assert!(at.distance(last) <= crate::glide::SPEED * FRAME + 1e-4);
            highest = highest.max(at.y);
            last = at;
        }
        assert!(highest > 2.5);
        assert_eq!(last, Vec3::new(2.0, 1.4, -2.3));
        // Turned to face someone, it turns about the vertical to face them, where it stands.
        conversation.apply(space(|state| {
            state.movement = Some(tomori_protocol::AgentMovement::default());
            state.summoned = Some(to);
            state.facing = Some([4.0, 1.6, 0.0]);
        }));
        let facing = conversation.arrangement().agent;
        assert_eq!(facing.position, Vec3::new(2.0, 1.4, -2.3));
        let ahead = facing.orientation * Vec3::Z;
        let towards = Vec3::new(2.0, 0.0, 2.3).normalize();
        assert!(ahead.dot(towards) > 0.999, "{ahead}");
        for _ in 0..(1.0 / FRAME) as usize {
            step.update(&senses(), &mut conversation, &mut felt(), now, FRAME);
        }
        let frame = step.update(&senses(), &mut conversation, &mut felt(), now, FRAME);
        assert!(
            frame
                .swarm
                .anchor()
                .orientation
                .angle_between(facing.orientation)
                < 0.01
        );
        // Somewhere it cannot face, or a waypoint beyond the space, is ignored; and far away it streams.
        conversation.apply(space(|state| {
            state.movement = Some(tomori_protocol::AgentMovement::default());
            state.summoned = Some(to);
            state.facing = Some([f32::NAN, 0.0, 0.0]);
            state.via = Some([0.0, 0.0, 900.0]);
            state.stream = true;
        }));
        let arrangement = conversation.arrangement();
        assert_eq!(
            arrangement.agent.orientation,
            crate::anchors::Anchor::from_pose(&to).unwrap().orientation
        );
        assert_eq!((arrangement.via, arrangement.stream), (None, true));
    }

    #[test]
    fn the_agent_appears_at_its_home_as_the_space_opens_and_never_moves_of_its_own_accord() {
        let now = Instant::now();
        let home = tomori_protocol::Pose::at([-1.5, 1.6, -3.5]).facing_towards([0.0, 1.6, 0.0]);
        let anchor = crate::anchors::Anchor::from_pose(&home).unwrap();
        let mut conversation = lobby(ConnectionRole::Owner);
        let mut step = Step::default();
        for _ in 0..200 {
            step.update(&senses(), &mut conversation, &mut felt(), now, FRAME);
        }
        let middle = |step: &Step| {
            step.coordinator
                .swarm()
                .agents
                .iter()
                .map(|agent| agent.position)
                .sum::<Vec3>()
                / crate::swarm::AGENT_COUNT as f32
        };
        let before = middle(&step) - crate::swarm::CENTER;
        conversation.apply(space(|state| {
            state.anchors = tomori_protocol::Anchors::new().with_agent(home);
        }));
        // In the first frame in the space, it is there, not gliding.
        let frame = step.update(&senses(), &mut conversation, &mut felt(), now, FRAME);
        assert_eq!(frame.swarm.anchor(), anchor);
        let appeared = middle(&step);
        assert!(appeared.distance(anchor.place(before)) < 0.2, "{appeared}");
        // And it stays, through a long idle, with no summons.
        for _ in 0..3000 {
            let frame = step.update(&senses(), &mut conversation, &mut felt(), now, FRAME);
            assert_eq!(frame.swarm.anchor(), anchor);
        }
        assert!(placed(&mut conversation).is_empty());
    }

    #[test]
    fn an_environment_arriving_elsewhere_is_carried_into_the_space_for_its_client_side_and_scenery()
    {
        use std::sync::{Arc, Mutex};
        // A client side that records the ray and head it is given, and makes a sound at its own origin.
        #[derive(Default)]
        struct Seen {
            rays: Vec<Ray>,
            heads: Vec<Vec3>,
        }
        struct Recording(Arc<Mutex<Seen>>);
        impl crate::client_side::ClientSide for Recording {
            fn pick(&self, ray: &Ray) -> Option<crate::client_side::Picked> {
                self.0.lock().unwrap().rays.push(*ray);
                None
            }
            fn update(
                &mut self,
                input: crate::client_side::Input<'_>,
            ) -> crate::client_side::Output {
                self.0.lock().unwrap().heads.extend(input.head);
                crate::client_side::Output::new().with_sound(
                    crate::client_side::Sound::new(vec![0.0; 4], 48_000),
                    [0.0, 0.0, 0.0],
                )
            }
        }
        let seen = Arc::new(Mutex::new(Seen::default()));
        let factory: crate::client_side::Factory = {
            let seen = seen.clone();
            Arc::new(move |_space: &str| {
                Box::new(Recording(seen.clone())) as Box<dyn crate::client_side::ClientSide>
            })
        };
        // Participants arrive 8 m along +X in the environment, facing its origin.
        let street = tomori_protocol::Pose::at([8.0, 0.0, 0.0]).facing_towards([0.0; 3]);
        let mut conversation = lobby(ConnectionRole::Owner);
        conversation.set_client_sides(vec![(crate::test_support::SKY, factory)]);
        conversation.apply(space(|state| {
            state.anchors = tomori_protocol::Anchors::new().with_arrival(street);
        }));
        let carriage = conversation.arrangement().carriage;
        assert!(
            carriage
                .place(Vec3::ZERO)
                .distance(Vec3::new(0.0, 0.0, -8.0))
                < 1e-4
        );
        let mut step = Step::default();
        let pointing = Senses {
            ray: Some(Ray {
                origin: Vec3::new(0.0, 1.5, 0.0),
                direction: Vec3::NEG_Z,
            }),
            beam: true,
            ..senses()
        };
        let mut feedback = felt();
        let mut frame_carriage = None;
        for _ in 0..2000 {
            let frame = step.update(
                &pointing,
                &mut conversation,
                &mut feedback,
                Instant::now(),
                FRAME,
            );
            frame_carriage = Some(frame.carriage);
        }
        // The scenery is drawn carried, and the client side picks and hears in the environment's own coordinates:
        // from the street, looking at its origin.
        assert_eq!(frame_carriage, Some(carriage));
        let seen = seen.lock().unwrap();
        let ray = seen.rays.last().expect("picked");
        assert!(
            ray.origin.distance(Vec3::new(8.0, 1.5, 0.0)) < 1e-4,
            "{:?}",
            ray.origin
        );
        assert!(
            ray.direction.distance(Vec3::NEG_X) < 1e-4,
            "{:?}",
            ray.direction
        );
        let head = seen.heads.last().expect("told where the head is");
        assert!(head.distance(Vec3::new(8.0, 1.6, 0.0)) < 1e-4, "{head}");
        drop(seen);
        // Its sound, made at its origin, is heard 8 m ahead of home.
        let (_, position) = feedback.take_ambient().next_back().expect("a sound");
        assert!(Vec3::from_array(position).distance(Vec3::new(0.0, 0.0, -8.0)) < 1e-4);
        // On the grid, nothing is carried.
        conversation.apply(space(|state| {
            state.environment = "minimal".into();
            state.anchors = tomori_protocol::Anchors::new().with_arrival(street);
        }));
        assert_eq!(
            conversation.arrangement().carriage,
            crate::anchors::Anchor::HOME
        );
    }

    #[test]
    fn leaving_a_space_keeps_its_environment_and_agent_where_they_were_seen_until_it_has_gone() {
        use crate::locomotion::{Locomotion, Surface};
        // Participants arrive 8 m along +X, facing the environment's origin, as the farmyard's arrive inside its gate;
        // the agent's home is its own; and travel teleports.
        let street = tomori_protocol::Pose::at([8.0, 0.0, 0.0]).facing_towards([0.0; 3]);
        let home = tomori_protocol::Pose::at([1.0, 1.4, -3.0]).facing_towards([0.0, 1.6, 0.0]);
        let middle = |step: &Step| {
            step.coordinator
                .swarm()
                .agents
                .iter()
                .map(|agent| agent.position)
                .sum::<Vec3>()
                / crate::swarm::AGENT_COUNT as f32
        };
        let close = |a: crate::anchors::Anchor, b: crate::anchors::Anchor| {
            a.position.distance(b.position) < 1e-4
                && a.orientation.dot(b.orientation).abs() > 1.0 - 1e-6
        };
        // Left with B held, as the frame's buttons are read before the platform places its poses, or within a frame
        // after it placed them, as the hand menu leaves.
        for within in [false, true] {
            let mut conversation = lobby(ConnectionRole::Owner);
            conversation.set_locomotion(vec![(
                crate::test_support::SKY,
                Locomotion::teleport(vec![Surface::rectangle(Vec3::ZERO, Vec2::splat(40.0))]),
            )]);
            conversation.apply(space(|state| {
                state.anchors = tomori_protocol::Anchors::new()
                    .with_arrival(street)
                    .with_agent(home);
            }));
            let arrangement = conversation.arrangement();
            assert_ne!(arrangement.carriage, crate::anchors::Anchor::HOME);
            let mut step = Step::default();
            let now = Instant::now();
            // The platform places the frame's poses with the rig, as it was settled before the frame.
            let placed = |step: &mut Step, conversation: &Conversation, ray: Vec3| {
                let rig = step.rig(conversation).matrix();
                let head = rig * Mat4::from_translation(arrival::HOME);
                Senses {
                    head: Some(head),
                    head_pose: Pose::from_transform(head),
                    viewer: Some(head.w_axis.truncate()),
                    ray: Ray::new(
                        rig.transform_point3(arrival::HOME),
                        rig.transform_vector3(ray),
                    ),
                    ..senses()
                }
            };
            let stick = |step: &mut Step, conversation: &mut Conversation, stick: Vec2| {
                step.buttons(
                    &press(|buttons| buttons.left_stick = stick),
                    true,
                    conversation,
                    &mut felt(),
                    now,
                );
            };
            // Entering is as it was: the environment is carried, and the agent stands at its home, from the first frame.
            for _ in 0..400 {
                let senses = placed(&mut step, &conversation, -Vec3::Z);
                let frame = step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
                assert_eq!(frame.carriage, arrangement.carriage);
                assert_eq!(frame.swarm.anchor(), arrangement.agent);
            }
            // Teleported down the yard, the space is drawn from there.
            stick(&mut step, &mut conversation, Vec2::Y);
            let senses = placed(&mut step, &conversation, Vec3::new(0.3, -0.5, -1.0));
            assert!(
                step.update(&senses, &mut conversation, &mut felt(), now, FRAME)
                    .landing
                    .is_some()
            );
            stick(&mut step, &mut conversation, Vec2::ZERO);
            step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
            let rig = step.rig(&conversation);
            assert_ne!(rig, Rig::default());
            let senses = placed(&mut step, &conversation, -Vec3::Z);
            let frame = step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
            assert_eq!(frame.carriage, arrangement.carriage);
            // Where the environment and the agent stand in the participant's room.
            let carriage = rig.unplace(arrangement.carriage);
            let agent = rig.unplace(arrangement.agent);
            let mut swarm = rig.matrix().inverse().transform_point3(middle(&step));
            // Leaving, the rig goes home at once, while the environment shrinks back into the space's object.
            let mut senses = if within {
                let senses = placed(&mut step, &conversation, -Vec3::Z);
                assert!(conversation.hold_back(Some(LEAVE_HOLD)));
                senses
            } else {
                assert!(conversation.hold_back(Some(LEAVE_HOLD)));
                let senses = placed(&mut step, &conversation, -Vec3::Z);
                assert_eq!(
                    step.placed,
                    Rig::default(),
                    "home before the frame is placed"
                );
                senses
            };
            let mut shrinking = 0;
            loop {
                let rig = step.placed;
                let frame = step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
                if frame.scenery.environment == crate::environment::Environment::MINIMAL {
                    // Once it has gone, the agent appears back at its place, and nothing is carried.
                    assert_eq!(frame.carriage, crate::anchors::Anchor::HOME);
                    assert_eq!(frame.swarm.anchor(), crate::anchors::Anchor::AGENT);
                    break;
                }
                shrinking += 1;
                // While it is drawn, it and the agent stay where the participant saw them, in their room.
                assert!(
                    close(rig.unplace(frame.carriage), carriage),
                    "{within} {shrinking}"
                );
                assert!(
                    close(rig.unplace(frame.swarm.anchor()), agent),
                    "{within} {shrinking}"
                );
                let seen = rig.matrix().inverse().transform_point3(middle(&step));
                assert!(
                    seen.distance(swarm) < 0.05,
                    "{within} {shrinking}: {seen} {swarm}"
                );
                swarm = seen;
                senses = placed(&mut step, &conversation, -Vec3::Z);
                assert_eq!(step.placed, Rig::default());
            }
            assert!(shrinking > 10, "it shrinks away over frames: {shrinking}");
            // And it stays there.
            let frame = step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
            assert_eq!(frame.swarm.anchor(), crate::anchors::Anchor::AGENT);
            assert_eq!(frame.carriage, crate::anchors::Anchor::HOME);
        }
    }

    #[test]
    fn the_ray_chooses_what_it_points_at_and_draws_the_beam_where_the_platform_does() {
        let now = Instant::now();
        for beam in [true, false] {
            let mut conversation = Conversation::new("Ada");
            conversation.set_servers(vec![tomori_client::server_choice::Choice::new(
                "Studio",
                "10.0.0.1:8765",
                tomori_client::server_choice::Source::Discovered,
            )]);
            let mut feedback = felt();
            let mut step = Step::default();
            // Arrival's objects move in over a few seconds.
            step.update(&senses(), &mut conversation, &mut feedback, now, FRAME);
            let frame = step.update(&senses(), &mut conversation, &mut feedback, now, 5.0);
            let scene = frame.arrival.expect("arrival's objects").scene;
            let target = target(&scene, &arrival::Key::Server("10.0.0.1:8765".into()));
            let aiming = Senses {
                ray: aim_at(target.shape),
                beam,
                ..senses()
            };
            let frame = step.update(&aiming, &mut conversation, &mut feedback, now, FRAME);
            let arrival = frame.arrival.expect("arrival");
            assert_eq!(arrival.hovered, Some(target.key.clone()));
            assert_eq!(arrival.ray.is_some_and(|ray| ray.hit), beam);
            assert!(earcons(&mut feedback).is_empty());
            // Pulling the trigger on it chooses it, and is heard.
            let pull = Senses {
                trigger: Some(1.0),
                ..aiming
            };
            let frame = step.update(&pull, &mut conversation, &mut feedback, now, FRAME);
            assert!(!frame.attracting);
            assert_eq!(earcons(&mut feedback), [Earcon::Choose]);
            assert!(
                conversation
                    .take_actions()
                    .iter()
                    .any(|action| matches!(action, Action::Connect(_)))
            );
        }
    }

    #[test]
    fn a_chooses_what_the_ray_met_last_frame_or_else_accepts() {
        let now = Instant::now();
        let mut conversation = Conversation::new("Ada");
        conversation.set_servers(vec![tomori_client::server_choice::Choice::new(
            "Studio",
            "10.0.0.1:8765",
            tomori_client::server_choice::Source::Discovered,
        )]);
        let mut feedback = felt();
        let mut step = Step::default();
        step.update(&senses(), &mut conversation, &mut feedback, now, FRAME);
        // With nothing highlighted, A accepts the selected server.
        step.buttons(
            &press(|buttons| buttons.a = true),
            true,
            &mut conversation,
            &mut feedback,
            now,
        );
        assert_eq!(earcons(&mut feedback), [Earcon::Choose]);
        assert!(
            conversation
                .take_actions()
                .iter()
                .any(|action| matches!(action, Action::Connect(_)))
        );
    }

    #[test]
    fn what_waits_is_cued_on_the_palms_only_where_the_platform_cues() {
        let now = Instant::now();
        // Palms turned down and away from the viewer, so that neither panel shows.
        let away = Palm {
            wrist: arrival::HOME + Vec3::new(0.0, -0.4, -0.3),
            normal: -Vec3::Y,
        };
        let cued = |conversation: &mut Conversation, cues: bool| {
            let senses = Senses {
                left_palm: Some(away),
                right_palm: Some(away),
                cues,
                ..senses()
            };
            Step::default()
                .update(&senses, conversation, &mut felt(), now, FRAME)
                .arrival
                .map_or(0, |arrival| arrival.scene.objects.len())
        };
        // A knock is cued on the left palm, and an approval on the right wrist.
        let mut knocked = joined();
        knock(&mut knocked, "knock-1", "Grace");
        assert_eq!(cued(&mut knocked, true), 1);
        assert_eq!(cued(&mut knocked, false), 0);
        let mut approval = approval_waiting();
        assert_eq!(cued(&mut approval, true), 1);
        assert_eq!(cued(&mut approval, false), 0);
        knock(&mut approval, "knock-1", "Grace");
        assert_eq!(cued(&mut approval, true), 2);
        assert_eq!(cued(&mut joined(), true), 0);
    }

    #[test]
    fn the_swarm_speaks_while_agent_speech_plays_though_the_agent_is_idle() {
        use tomori_client::connection::{Report, Session};
        let now = Instant::now();
        let mut conversation = joined();
        conversation.apply(space(|state| {
            state.activity = tomori_protocol::Activity::Idle;
        }));
        let mut step = Step::default();
        let speech = |conversation: &mut Conversation, playing| {
            Session::report(
                conversation,
                Report::AgentSpeech {
                    playing,
                    level: if playing { 0.3 } else { 0.0 },
                },
            );
        };
        speech(&mut conversation, true);
        let frame = step.update(&senses(), &mut conversation, &mut felt(), now, FRAME);
        assert_eq!(frame.swarm.visual.mode, interaction::Mode::Speaking);
        // Speech stopping for a moment, as a buffer runs dry, keeps it speaking.
        speech(&mut conversation, false);
        let frame = step.update(&senses(), &mut conversation, &mut felt(), now, FRAME);
        assert_eq!(frame.swarm.visual.mode, interaction::Mode::Speaking);
        // Once it has stopped for the hold, the swarm shows the agent as it is.
        for _ in 0..(interaction::SPEAKING_HOLD / FRAME) as usize + 1 {
            step.update(&senses(), &mut conversation, &mut felt(), now, FRAME);
        }
        let frame = step.update(&senses(), &mut conversation, &mut felt(), now, FRAME);
        assert_eq!(frame.swarm.visual.mode, interaction::Mode::Ready);
    }

    #[test]
    fn participants_report_their_head_and_hands_and_hear_from_their_head() {
        let now = Instant::now();
        let controller = Pose {
            position: [0.2, 1.1, -0.3],
            orientation: [0.0, 0.0, 0.0, 1.0],
        };
        let senses = Senses {
            controllers: [None, Some(controller)],
            ..senses()
        };
        let mut conversation = joined();
        let mut step = Step::default();
        let frame = step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        let presence = frame.presence.expect("a participant is seen");
        assert_eq!(Some(presence.head), senses.head_pose);
        assert_eq!((presence.left, presence.right), (None, Some(controller)));
        assert_eq!(frame.listener, senses.head_pose);
        // The agent is heard from the swarm's middle.
        assert!(Vec3::from_array(frame.agent).distance(crate::swarm::CENTER) < 1.0);
        // Without a head there is nothing to report, and visitors report nothing.
        let headless = Senses {
            head_pose: None,
            ..senses
        };
        assert!(
            step.update(&headless, &mut conversation, &mut felt(), now, FRAME)
                .presence
                .is_none()
        );
        let mut visitor = lobby(ConnectionRole::Visitor);
        visitor.apply(space(|_| {}));
        let mut step = Step::default();
        assert!(
            step.update(&senses, &mut visitor, &mut felt(), now, FRAME)
                .presence
                .is_none()
        );
    }

    #[test]
    fn the_swarm_and_workers_stand_where_the_space_s_environment_puts_them() {
        use crate::anchors::{Anchor, Placing, WorkerPlacement};
        struct Ahead;
        impl WorkerPlacement for Ahead {
            fn place(&self, origin: Anchor, worker: &Placing<'_>) -> Vec3 {
                origin.place(Vec3::new(worker.slot as f32, 1.5, -2.0))
            }
        }
        let agent = tomori_protocol::Pose {
            position: [3.0, 1.6, -5.0],
            orientation: [0.0, 0.0, 0.0, 1.0],
        };
        let workers = tomori_protocol::Pose {
            position: [0.0, 0.0, -4.0],
            orientation: [0.0, 0.0, 0.0, 1.0],
        };
        let mut conversation = lobby(ConnectionRole::Owner);
        conversation
            .set_worker_placements(vec![(crate::test_support::SKY, std::sync::Arc::new(Ahead))]);
        conversation.apply(space(|state| {
            state.anchors = tomori_protocol::Anchors::new()
                .with_agent(agent)
                .with_workers(workers);
            state.workers = vec![tomori_protocol::Worker {
                id: "ses_a".into(),
                parent: None,
                title: "Review".into(),
                state: tomori_protocol::WorkerState::Running,
            }];
        }));
        let mut step = Step::default();
        let now = Instant::now();
        // From the first frame in the space, while its scenery still grows in, the placement places the worker.
        let first = step
            .update(&senses(), &mut conversation, &mut felt(), now, FRAME)
            .stations[0]
            .position;
        let scenery = step.motion.scenery(step.arrival_time);
        assert!(
            scenery.reveal.is_some()
                || scenery.environment == crate::environment::Environment::MINIMAL,
            "still growing in"
        );
        assert_eq!(first, [0.0, 1.5, -6.0]);
        for _ in 0..1000 {
            step.update(&senses(), &mut conversation, &mut felt(), now, FRAME);
        }
        let frame = step.update(&senses(), &mut conversation, &mut felt(), now, FRAME);
        assert_eq!(frame.swarm.anchor(), Anchor::from_pose(&agent).unwrap());
        // The agent is heard from the swarm, where it has glided.
        assert!(Vec3::from_array(frame.agent).distance(Vec3::from_array(agent.position)) < 1.0);
        // The worker stands where the placement put it about the workers anchor.
        assert_eq!(frame.stations[0].position, [0.0, 1.5, -6.0]);
    }

    #[test]
    fn the_avatar_the_space_s_backend_names_stands_in_for_the_swarm_from_the_first_frame() {
        use crate::avatar::{Avatar, Factory, Input};
        use std::sync::{Arc, Mutex};
        /// A figure standing at its anchor: heard from its mouth, workers leaving from its hand.
        struct Figure(Arc<Mutex<Vec<Input>>>, crate::anchors::Anchor);
        impl Avatar for Figure {
            fn update(&mut self, input: &Input) {
                self.1 = input.anchor;
                self.0.lock().unwrap().push(*input);
            }
            fn voices(&self, voices: &mut Vec<[f32; 3]>) {
                voices.push((self.1.position + Vec3::Y * 0.3).to_array());
            }
            fn departures(&self, departures: &mut Vec<[f32; 3]>) {
                departures.push((self.1.position + Vec3::X * 0.4).to_array());
            }
            fn bounds(&self) -> crate::arrival_motion::Sphere {
                crate::arrival_motion::Sphere {
                    centre: self.1.position,
                    radius: 0.5,
                }
            }
            fn drawn(&self) -> Option<crate::client_side::Drawn> {
                Some(Arc::new(self.1.position))
            }
        }
        let told = Arc::new(Mutex::new(Vec::new()));
        let made = Arc::new(Mutex::new(Vec::new()));
        let factory: Factory = {
            let (told, made) = (told.clone(), made.clone());
            Arc::new(move |space: &str| {
                made.lock().unwrap().push(space.to_owned());
                Box::new(Figure(told.clone(), crate::anchors::Anchor::AGENT)) as Box<dyn Avatar>
            })
        };
        let agent = tomori_protocol::Pose {
            position: [3.0, 0.0, -5.0],
            orientation: [0.0, 0.0, 0.0, 1.0],
        };
        let figure = |state: &mut tomori_protocol::SpaceState| {
            state.avatar = Some("figure".into());
            state.anchors = tomori_protocol::Anchors::new().with_agent(agent);
        };
        let mut conversation = lobby(ConnectionRole::Owner);
        conversation.set_avatars(vec![("figure".into(), factory)]);
        let mut step = Step::default();
        let now = Instant::now();
        // Outside a space, the swarm.
        let frame = step.update(&senses(), &mut conversation, &mut felt(), now, FRAME);
        assert!(frame.avatar.is_none());
        assert_eq!(frame.swarm.agents.len(), crate::swarm::AGENT_COUNT);
        // From the first frame in the space, while its scenery still grows in, the figure stands in for the swarm.
        conversation.apply(space(figure));
        let frame = step.update(&senses(), &mut conversation, &mut felt(), now, FRAME);
        let shown = frame.avatar.as_ref().expect("the figure shows");
        assert_eq!(shown.avatar, "figure");
        let position = *shown
            .drawn
            .as_ref()
            .unwrap()
            .downcast_ref::<Vec3>()
            .unwrap();
        assert_eq!(position, Vec3::from_array(agent.position));
        // The renderer draws no swarm, though the ring beneath the agent stands where it does.
        assert!(frame.swarm.agents.is_empty());
        assert_eq!(frame.swarm.anchor().position, position);
        // The agent is heard from its mouth, and a worker that arrives flies out from its hand.
        assert_eq!(frame.agent, [3.0, 0.3, -5.0]);
        conversation.apply(space(|state| {
            figure(state);
            state.workers = vec![tomori_protocol::Worker {
                id: "ses_a".into(),
                parent: None,
                title: "Review".into(),
                state: tomori_protocol::WorkerState::Running,
            }];
        }));
        let frame = step.update(&senses(), &mut conversation, &mut felt(), now, FRAME);
        let hand = Vec3::new(3.4, 0.0, -5.0);
        let leaving = frame
            .cubes
            .iter()
            .filter(|cube| Vec3::from_array(cube.position).distance(hand) < 0.2)
            .count();
        assert!(leaving > 0, "{:?}", frame.cubes);
        // It was told the agent's state, where it stands and where the participant's head is.
        let input = *told.lock().unwrap().last().unwrap();
        assert_eq!(input.anchor.position, position);
        assert_eq!(input.head, Some(arrival::HOME));
        assert!(!input.speaking);
        assert_eq!(input.attention, None);
        assert_eq!(input.dt, FRAME);
        assert_eq!(input.attractor, None);
        // The agent at work, and the trigger pulled away from everything, which in a space draws nothing: it is told
        // the one and not the other.
        conversation.apply(space(|state| {
            figure(state);
            state.activity = tomori_protocol::Activity::Running;
        }));
        let pull = Vec3::new(1.0, 1.2, -1.5);
        let pulling = Senses {
            ray: Ray::new(arrival::HOME, Vec3::Y),
            trigger: Some(1.0),
            attractor: Some(pull),
            ..senses()
        };
        for _ in 0..10 {
            step.update(&pulling, &mut conversation, &mut felt(), now, FRAME);
        }
        let input = *told.lock().unwrap().last().unwrap();
        assert_eq!(input.attractor, None);
        assert_eq!(input.visual.mode, interaction::Mode::Working);
        assert!(input.visual.working);
        // Made once, for the space, which it stands in.
        assert_eq!(*made.lock().unwrap(), [SPACE]);
        assert_eq!(step.coordinator.bounds().radius, 0.5);
        // Leaving, the swarm again, where it glided meanwhile.
        conversation.apply(tomori_client::Event::Message(
            tomori_protocol::ServerMessage::LEFT,
        ));
        let frame = step.update(&senses(), &mut conversation, &mut felt(), now, FRAME);
        assert!(frame.avatar.is_none());
        assert_eq!(frame.swarm.agents.len(), crate::swarm::AGENT_COUNT);
    }

    #[test]
    fn the_pill_stands_ahead_of_the_head_and_a_recentre_places_it_again() {
        let now = Instant::now();
        let mut conversation = joined();
        let mut step = Step::default();
        let frame = step.update(&senses(), &mut conversation, &mut felt(), now, FRAME);
        let model = frame.panels.pill.model.expect("the pill shows in a space");
        let ahead = model.transform_point3(Vec3::ZERO);
        assert!(ahead.z < arrival::HOME.z - 1.0, "{ahead}");
        // Turned round, the pill follows only lazily, until a recentre places it ahead again.
        let turned = Senses {
            space_head: Mat4::from_rotation_translation(
                glam::Quat::from_rotation_y(std::f32::consts::PI),
                arrival::HOME,
            ),
            ..senses()
        };
        let frame = step.update(&turned, &mut conversation, &mut felt(), now, FRAME);
        let lazily = frame
            .panels
            .pill
            .model
            .unwrap()
            .transform_point3(Vec3::ZERO);
        assert!(lazily.z < arrival::HOME.z, "{lazily}");
        step.recentre();
        let frame = step.update(&turned, &mut conversation, &mut felt(), now, FRAME);
        let placed = frame
            .panels
            .pill
            .model
            .unwrap()
            .transform_point3(Vec3::ZERO);
        assert!(placed.z > arrival::HOME.z + 1.0, "{placed}");
    }

    #[test]
    fn the_transcript_takes_the_right_stick_a_b_and_the_grip_while_it_shows() {
        let mut conversation = approval_waiting();
        let mut step = Step::default();
        let start = Instant::now();
        let open = Senses {
            right_palm: Some(palm_up(0.0)),
            ..senses()
        };
        assert!(
            wrist_after(&mut step, &mut conversation, &open, start)
                .model
                .is_some()
        );
        let mut feedback = felt();
        let clock = std::cell::Cell::new(start + HOLD);
        let tap = |step: &mut Step,
                   conversation: &mut Conversation,
                   feedback: &mut Feedback,
                   buttons: Buttons| {
            let now = clock.get() + Duration::from_millis(50);
            clock.set(now);
            step.buttons(&buttons, true, conversation, feedback, now);
            step.buttons(&Buttons::default(), true, conversation, feedback, now);
            now
        };
        // The stick moves the focus, which ticks on the right hand, instead of paging result cards.
        let moved = tap(
            &mut step,
            &mut conversation,
            &mut feedback,
            press(|buttons| buttons.right_stick = Vec2::new(0.0, -1.0)),
        );
        assert!(!felt_on(&mut feedback, Side::Right, moved).is_empty());
        assert_eq!(step.presentation_navigation.card, 0);
        // A expands the agent's turn, which is heard.
        let mut conversation = conversed();
        let mut step = Step::default();
        assert!(
            wrist_after(&mut step, &mut conversation, &open, start)
                .model
                .is_some()
        );
        tap(
            &mut step,
            &mut conversation,
            &mut feedback,
            press(|buttons| buttons.right_stick = Vec2::new(0.0, 1.0)),
        );
        // Sideways, it jumps between turns.
        let turn = conversation.wrist_focus().cloned();
        assert!(turn.is_some());
        tap(
            &mut step,
            &mut conversation,
            &mut feedback,
            press(|buttons| buttons.right_stick = Vec2::new(-1.0, 0.0)),
        );
        assert_ne!(conversation.wrist_focus().cloned(), turn);
        tap(
            &mut step,
            &mut conversation,
            &mut feedback,
            press(|buttons| buttons.right_stick = Vec2::new(1.0, 0.0)),
        );
        assert_eq!(conversation.wrist_focus().cloned(), turn);
        earcons(&mut feedback);
        tap(
            &mut step,
            &mut conversation,
            &mut feedback,
            press(|buttons| buttons.a = true),
        );
        assert_eq!(earcons(&mut feedback), [Earcon::Choose]);
        // The grip detaches it where it stands; with no head to follow, it stays.
        let detach = Senses {
            grip: Some(1.0),
            ..open
        };
        let now = clock.get();
        let frame = step.update(&detach, &mut conversation, &mut feedback, now, FRAME);
        let model = frame.panels.wrist.model;
        assert!(step.wrist.state().detached());
        let headless = Senses {
            head: None,
            grip: Some(0.0),
            right_palm: None,
            ..open
        };
        let frame = step.update(&headless, &mut conversation, &mut feedback, now, FRAME);
        assert_eq!(frame.panels.wrist.model, model);
        // B backs out of the expanded turn first, then returns it to the wrist, rather than backing out of the space.
        tap(
            &mut step,
            &mut conversation,
            &mut feedback,
            press(|buttons| buttons.back = true),
        );
        assert!(step.wrist.state().detached());
        tap(
            &mut step,
            &mut conversation,
            &mut feedback,
            press(|buttons| buttons.back = true),
        );
        assert!(!step.wrist.state().detached());
        assert!(conversation.joined_space().is_some());
    }

    #[test]
    fn the_ray_and_a_choose_only_in_the_hand_menu_while_it_shows() {
        use crate::hand_menu::MenuKey;
        let start = Instant::now();
        let menu = Senses {
            left_palm: Some(left_palm_up(0.0)),
            ..senses()
        };
        // Y pins the menu open at its Space section, whose rows choose at once.
        let open = |conversation: &mut Conversation, feedback: &mut Feedback| {
            let mut step = Step::default();
            step.buttons(
                &press(|buttons| buttons.y = true),
                true,
                conversation,
                feedback,
                start,
            );
            let frame = step.update(&menu, conversation, feedback, start, FRAME);
            let scene = frame.arrival.expect("the pinned menu").scene;
            (step, scene)
        };
        let mut conversation = joined();
        let mut feedback = felt();
        let (mut step, scene) = open(&mut conversation, &mut feedback);
        let mute = arrival::Key::Menu(MenuKey::Mute);
        let aiming = Senses {
            ray: aim_at(target(&scene, &mute).shape),
            beam: true,
            ..menu
        };
        let frame = step.update(&aiming, &mut conversation, &mut feedback, start, FRAME);
        let arrival = frame.arrival.expect("the menu");
        assert_eq!(arrival.hovered, Some(mute.clone()));
        assert!(arrival.ray.is_some_and(|ray| ray.hit));
        // Pointing at the menu counts as pointing, which keeps the transcript from showing.
        assert!(step.pointing(None));
        earcons(&mut feedback);
        // A chooses the row the ray is on.
        let muted = conversation.menu_model().muted;
        step.buttons(
            &press(|buttons| buttons.a = true),
            true,
            &mut conversation,
            &mut feedback,
            start,
        );
        assert_eq!(earcons(&mut feedback), [Earcon::Choose]);
        assert_ne!(conversation.menu_model().muted, muted);
        // As does the trigger, which never attracts while the menu shows.
        let mut conversation = joined();
        let (mut step, _) = open(&mut conversation, &mut feedback);
        step.update(&aiming, &mut conversation, &mut feedback, start, FRAME);
        let pull = Senses {
            trigger: Some(1.0),
            ..aiming
        };
        let frame = step.update(&pull, &mut conversation, &mut feedback, start, FRAME);
        assert!(!frame.attracting);
        assert_eq!(earcons(&mut feedback), [Earcon::Choose]);
        assert_ne!(conversation.menu_model().muted, muted);
        // The trigger on a section opens it, which is heard too.
        let mut conversation = joined();
        let mut step = Step::default();
        step.update(&menu, &mut conversation, &mut feedback, start, FRAME);
        let frame = step.update(&menu, &mut conversation, &mut feedback, start + HOLD, FRAME);
        let section = arrival::Key::Menu(MenuKey::Section(hand_menu::Section::Sound));
        let aiming = Senses {
            ray: aim_at(target(&frame.arrival.expect("the menu").scene, &section).shape),
            ..menu
        };
        step.update(
            &aiming,
            &mut conversation,
            &mut feedback,
            start + HOLD,
            FRAME,
        );
        let pull = Senses {
            trigger: Some(1.0),
            ..aiming
        };
        let frame = step.update(&pull, &mut conversation, &mut feedback, start + HOLD, FRAME);
        assert_eq!(earcons(&mut feedback), [Earcon::Choose]);
        let rows = frame.arrival.expect("the menu").scene;
        assert!(
            rows.object(&arrival::Key::Menu(MenuKey::Sounds)).is_some(),
            "the Sound section is open"
        );
    }

    /// Hold the ray on the menu's row `key`, as `menu` shows it, for a second of frames, through which it stays on that row; then pull the trigger, which chooses it, and let go.
    fn hold_and_choose(
        step: &mut Step,
        conversation: &mut Conversation,
        menu: &Senses,
        key: hand_menu::MenuKey,
        now: Instant,
    ) {
        let key = arrival::Key::Menu(key);
        let mut feedback = felt();
        let frame = step.update(menu, conversation, &mut feedback, now, FRAME);
        let scene = frame.arrival.expect("the menu").scene;
        let aiming = Senses {
            ray: aim_at(target(&scene, &key).shape),
            ..*menu
        };
        for frame in 0..72 {
            let hovered = step
                .update(&aiming, conversation, &mut feedback, now, FRAME)
                .arrival
                .expect("the menu")
                .hovered;
            assert_eq!(hovered.as_ref(), Some(&key), "frame {frame}");
        }
        let pull = Senses {
            trigger: Some(1.0),
            ..aiming
        };
        step.update(&pull, conversation, &mut feedback, now, FRAME);
        assert_eq!(earcons(&mut feedback), [Earcon::Choose], "{key:?}");
        step.update(&aiming, conversation, &mut feedback, now, FRAME);
    }

    #[test]
    fn a_steady_ray_stays_on_a_menu_row_and_chooses_it() {
        use crate::hand_menu::{MenuKey, Section};
        use tomori_client::{ConnectionState, Event};
        use tomori_protocol::{ClientMessage, EnvironmentInfo, Mode};
        // An owner, in a space whose agent has three modes, each described, the middle one current, on a server that changes environments.
        let mut conversation = lobby(ConnectionRole::Owner);
        conversation.apply(Event::State(ConnectionState::Connected {
            source: 1,
            server: "studio".into(),
            role: ConnectionRole::Owner,
            certificate: None,
            backends: vec![],
            roots: vec![],
            environments: [("sky", "Sky"), ("minimal", "Grid")]
                .map(|(id, label)| EnvironmentInfo {
                    id: id.into(),
                    label: label.into(),
                    version: 1,
                })
                .to_vec(),
            utterance_limit: None,
            name: None,
        }));
        conversation.apply(space(|state| {
            state.modes = [
                (
                    "plan",
                    "Plans without changing files, and says how it would.",
                ),
                ("build", "Makes changes."),
                ("review", "Reviews the changes made so far."),
            ]
            .map(|(id, description)| Mode {
                id: id.into(),
                name: id.into(),
                description: Some(description.into()),
            })
            .to_vec();
            state.mode = Some("build".into());
            state.speech = tomori_protocol::SpeechSetting::Brief;
            state.environment_changes = true;
        }));
        conversation.take_actions();
        let start = Instant::now();
        let menu = Senses {
            left_palm: Some(left_palm_up(0.0)),
            ..senses()
        };
        let mut step = Step::default();
        step.buttons(
            &press(|buttons| buttons.y = true),
            true,
            &mut conversation,
            &mut felt(),
            start,
        );
        // The modes above and below the current one, each held for a second, stay highlighted and are chosen; pointing at one shows its description beneath the modes without moving a row.
        let sent = |conversation: &mut Conversation, message: ClientMessage| {
            assert_eq!(conversation.take_actions(), [Action::Send(message)]);
        };
        for mode in ["plan", "review"] {
            hold_and_choose(
                &mut step,
                &mut conversation,
                &menu,
                MenuKey::Mode(mode.into()),
                start,
            );
            sent(
                &mut conversation,
                ClientMessage::SetMode { mode: mode.into() },
            );
        }
        // As do the environment the space is not in, and the space's other settings.
        hold_and_choose(
            &mut step,
            &mut conversation,
            &menu,
            MenuKey::Environment("minimal".into()),
            start,
        );
        sent(
            &mut conversation,
            ClientMessage::SetEnvironment {
                environment: "minimal".into(),
            },
        );
        hold_and_choose(&mut step, &mut conversation, &menu, MenuKey::Speech, start);
        sent(
            &mut conversation,
            ClientMessage::SetSpeech {
                speech: tomori_protocol::SpeechSetting::Full,
            },
        );
        // The Sound section's levels, stepped one after another.
        hold_and_choose(&mut step, &mut conversation, &menu, MenuKey::Back, start);
        hold_and_choose(
            &mut step,
            &mut conversation,
            &menu,
            MenuKey::Section(Section::Sound),
            start,
        );
        let feedback = conversation.menu_model().feedback;
        for key in [MenuKey::Sounds, MenuKey::Sounds, MenuKey::Vibration] {
            hold_and_choose(&mut step, &mut conversation, &menu, key, start);
        }
        let stepped = conversation.menu_model().feedback;
        assert_eq!(stepped.sounds, feedback.sounds.next().next());
        assert_eq!(stepped.vibration, feedback.vibration.next());
    }

    #[test]
    fn while_the_menu_shows_in_arrival_the_ray_meets_only_its_rows() {
        let mut conversation = Conversation::new("Ada");
        conversation.set_servers(vec![tomori_client::server_choice::Choice::new(
            "Studio",
            "10.0.0.1:8765",
            tomori_client::server_choice::Source::Discovered,
        )]);
        let mut step = Step::default();
        let now = Instant::now();
        step.update(&senses(), &mut conversation, &mut felt(), now, FRAME);
        let frame = step.update(&senses(), &mut conversation, &mut felt(), now, 5.0);
        let server = target(
            &frame.arrival.expect("arrival").scene,
            &arrival::Key::Server("10.0.0.1:8765".into()),
        );
        let menu = Senses {
            left_palm: Some(left_palm_up(0.0)),
            ray: aim_at(server.shape),
            ..senses()
        };
        step.update(&menu, &mut conversation, &mut felt(), now, FRAME);
        let frame = step.update(&menu, &mut conversation, &mut felt(), now + HOLD, FRAME);
        let hovered = frame.arrival.expect("arrival and the menu").hovered;
        assert!(
            !matches!(hovered, Some(arrival::Key::Server(_))),
            "{hovered:?}"
        );
        // Nor does A choose the server behind it.
        step.buttons(
            &press(|buttons| buttons.a = true),
            true,
            &mut conversation,
            &mut felt(),
            now + HOLD,
        );
        assert!(conversation.take_actions().is_empty());
    }

    #[test]
    fn a_with_nothing_to_choose_is_not_heard() {
        let mut conversation = joined();
        let mut feedback = felt();
        Step::default().buttons(
            &press(|buttons| buttons.a = true),
            true,
            &mut conversation,
            &mut feedback,
            Instant::now(),
        );
        assert!(earcons(&mut feedback).is_empty());
    }

    #[test]
    fn in_a_space_the_beam_ends_on_the_nearest_wall_handle_panel_or_object_it_meets() {
        use tomori_protocol::{Panel, Place, Pose, Wall, WallKind};
        let beam = |frame: &Frame<'_>| {
            frame
                .arrival
                .as_ref()
                .and_then(|arrival| arrival.ray)
                .expect("the beam")
        };
        // An environment's wall ahead, to the right, and a panel floating ahead, to the left, nearer.
        let wall = Wall::new(
            "environment-0",
            WallKind::Environment,
            Pose::upright_facing([1.2, 1.5, -2.5], arrival::HOME.to_array()),
        );
        let floating = arrival::HOME + Vec3::new(-0.8, 0.0, -1.5);
        let panel = Panel::new(
            "panel-1",
            "Notes",
            "Ahead.",
            Place::Free {
                pose: Pose::upright_facing(floating.to_array(), arrival::HOME.to_array()),
            },
        );
        let board = Vec3::new(1.2, 1.5 + 0.42, -2.5);
        let run = |role: ConnectionRole| {
            let mut conversation = lobby(role);
            fixtures::show_editable_walls(
                &mut conversation,
                vec![wall.clone()],
                vec![panel.clone()],
            );
            (conversation, Step::default())
        };
        let now = Instant::now();
        let mut senses = Senses {
            beam: true,
            grip: Some(0.0),
            ..senses()
        };
        let (mut conversation, mut step) = run(ConnectionRole::Owner);
        // At the wall's board, which a participant may take hold of: it ends there, as it does on a panel.
        senses.ray = Ray::new(arrival::HOME, board - arrival::HOME);
        let frame = step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        let ends = beam(&frame);
        assert!(
            ends.hit && ends.end.distance(board) < 1e-3,
            "{:?}",
            ends.end
        );
        senses.ray = Ray::new(arrival::HOME, floating - arrival::HOME);
        let frame = step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        let ends = beam(&frame);
        assert!(
            ends.hit && ends.end.distance(floating) < 1e-3,
            "{:?}",
            ends.end
        );
        // At the wall's bar, above it, which takes hold of it as its board does: it ends there.
        let bar = crate::projection::editing::bar(
            &wall,
            crate::board::model(&wall.pose).unwrap(),
            arrival::HOME,
        );
        senses.ray = Ray::new(arrival::HOME, bar.middle - arrival::HOME);
        let frame = step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        let ends = beam(&frame);
        assert!(
            ends.hit && ends.end.distance(bar.middle) < 0.01,
            "{:?} {:?}",
            ends.end,
            bar.middle
        );
        // The bar is Tomori's, so the environment takes neither the trigger nor a stick there.
        assert_eq!(step.aim, precedence::Aim::Tomori);
        // Taken hold of, at a handle, at the handle, however near the board behind it.
        senses.ray = Ray::new(arrival::HOME, board - arrival::HOME);
        senses.grip = Some(1.0);
        step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        senses.grip = Some(0.0);
        step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        assert_eq!(step.projector.editing_wall(), Some("environment-0"));
        let frame = crate::board::model(&wall.pose).unwrap();
        let (corner, _) = crate::projection::editing::handle_point(
            &wall,
            frame,
            crate::projection::editing::Handle::Corner(1, 1),
            arrival::HOME,
        );
        senses.ray = Ray::new(arrival::HOME, corner - arrival::HOME);
        let drawn = step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        let ends = beam(&drawn);
        assert!(
            ends.hit && ends.end.distance(corner) < 1e-3,
            "{:?} {corner}",
            ends.end
        );
        // A visitor takes hold of no wall: the ray passes its board and ends short of it.
        let (mut conversation, mut step) = run(ConnectionRole::Visitor);
        senses.ray = Ray::new(arrival::HOME, board - arrival::HOME);
        let frame = step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        assert!(!beam(&frame).hit);
        // An environment's object a metre away, nearer than the wall, ends it there.
        let (mut conversation, mut step) = run(ConnectionRole::Owner);
        let told = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let factory: crate::client_side::Factory = std::sync::Arc::new(move |_: &str| {
            Box::new(Fake {
                frames: 0,
                told: told.clone(),
            }) as Box<dyn crate::client_side::ClientSide>
        });
        conversation.set_client_sides(vec![(crate::test_support::SKY, factory)]);
        conversation.apply(space(|state| {
            state.projection = true;
            state.walls = true;
        }));
        for _ in 0..400 {
            step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        }
        let frame = step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        let ends = beam(&frame);
        let ray = senses.ray.unwrap();
        assert!(
            ends.hit && ends.end.distance(ray.at(1.0)) < 1e-3,
            "{:?}",
            ends.end
        );
    }

    #[test]
    fn in_a_space_the_beam_reaches_out_and_ends_short_of_nothing() {
        let mut conversation = joined();
        let mut step = Step::default();
        let ray = Ray::new(arrival::HOME, -Vec3::Z);
        let pointing = Senses {
            ray,
            beam: true,
            ..senses()
        };
        let frame = step.update(
            &pointing,
            &mut conversation,
            &mut felt(),
            Instant::now(),
            FRAME,
        );
        let beam = frame
            .arrival
            .and_then(|arrival| arrival.ray)
            .expect("the beam");
        assert!(!beam.hit);
        assert_eq!(beam.end, ray.unwrap().at(crate::pointing::SPACE_RAY_LENGTH));
    }

    #[test]
    fn the_recording_indicator_and_the_live_badge_show_on_the_left() {
        let mut conversation = joined();
        let mut step = Step::default();
        let now = Instant::now();
        let grip = Pose {
            position: [-0.2, 1.1, -0.35],
            orientation: [0.0, 0.0, 0.0, 1.0],
        };
        let senses = Senses {
            trackers: Trackers {
                grip: Some(grip),
                ..Default::default()
            },
            ..senses()
        };
        let frame = step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        assert_eq!(frame.panels.indicator, [Sheet::default(), Sheet::default()]);
        conversation.toggle_live();
        assert!(conversation.live());
        let frame = step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        let [card, badge] = &frame.panels.indicator;
        assert_eq!(card, &Sheet::default());
        assert!(
            badge.canvas.is_some() && badge.model.is_some(),
            "the live badge"
        );
        conversation.stop_live();
        step.buttons(
            &press(|buttons| buttons.talk = true),
            true,
            &mut conversation,
            &mut felt(),
            now,
        );
        let frame = step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        let [card, badge] = &frame.panels.indicator;
        assert!(card.canvas.is_some(), "the recording card");
        // It rises from the left controller.
        let at = card.model.expect("drawn").transform_point3(Vec3::ZERO);
        assert!(at.distance(Vec3::from_array(grip.position)) < 0.2, "{at}");
        assert!(at.y > grip.position[1]);
        assert_eq!(badge, &Sheet::default());
        // With the live microphone on too, only the card shows.
        conversation.toggle_live();
        let frame = step.update(&senses, &mut conversation, &mut felt(), now, FRAME);
        let [card, badge] = &frame.panels.indicator;
        assert!(card.model.is_some());
        assert_eq!(badge, &Sheet::default());
    }

    /// The right stick's push for an object of an environment's that takes it.
    mod scroll;
}
