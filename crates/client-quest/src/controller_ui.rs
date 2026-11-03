// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The panels held by the controllers, shared by the headset's and the desktop window's frame loops: the recording indicator and the `LIVE` badge on the left controller (see `design/hand-menu.md`), and the transcript on the right wrist (see `design/wrist-transcript.md`).

use std::time::{Duration, Instant};

use glam::{Mat4, Vec2, Vec3};

use crate::conversation::Conversation;
use crate::graphics::Renderer;
use crate::hand_menu::{Palm, Visibility};
use crate::panel_anchor::PanelAnchor;
use crate::recording::{self, Anchor, Trackers};
use crate::transcript::{DETACHED_COLUMNS, WRIST_COLUMNS};
use crate::ui_panel::{Canvas, Panel};

/// When a frame is, for the panels that move and animate.
#[derive(Clone, Copy, Debug)]
pub struct Moment {
    pub now: Instant,
    /// Seconds since the last frame.
    pub dt: f32,
    /// Seconds since the client started, for animation.
    pub time: f32,
}

/// Each panel's most vertices.
const CAPACITY: usize = 24_000;

/// The recording indicator and the `LIVE` badge, with where they are.
pub struct Indicator {
    card: Panel,
    badge: Panel,
    anchor: Anchor,
    badge_anchor: Anchor,
    /// The card's and the badge's models in the scene, while shown.
    shown: [Option<Mat4>; 2],
}

impl Indicator {
    pub fn new(device: &wgpu::Device, renderer: &Renderer) -> Self {
        Self {
            card: renderer.panel(device, CAPACITY),
            badge: renderer.panel(device, CAPACITY / 8),
            anchor: Anchor::default(),
            badge_anchor: Anchor::default(),
            shown: [None, None],
        }
    }

    /// Follow the conversation at `moment`, placing the indicator by `trackers` facing `head`, both in the scene.
    pub fn update(
        &mut self,
        queue: &wgpu::Queue,
        conversation: &Conversation,
        trackers: Trackers,
        head: Option<Vec3>,
        moment: Moment,
    ) {
        let Moment { now, dt, time } = moment;
        let pulse = 0.5 + 0.5 * (time * 5.0).sin();
        let phase = conversation.recording_phase(now);
        let head = head.unwrap_or(crate::arrival::HOME);
        self.shown[0] = match (&phase, trackers.target(recording::GRIP_OFFSET)) {
            (Some(phase), Some(target)) => {
                let canvas = recording::canvas(phase, conversation.utterance_limit(), pulse);
                self.card.set(queue, &canvas);
                let (position, scale) = self.anchor.update(target, dt);
                Some(recording::facing(position, head, scale))
            }
            _ => {
                self.anchor.hide();
                None
            }
        };
        // The badge stays on the controller while the live microphone is on, except while the card shows.
        let badge = trackers
            .grip
            .and_then(|_| trackers.target(recording::BADGE_OFFSET));
        self.shown[1] = match badge {
            Some(target) if conversation.live() && self.shown[0].is_none() => {
                self.badge.set(queue, &recording::badge(pulse));
                let (position, scale) = self.badge_anchor.update(target, dt);
                Some(recording::facing(position, head, scale))
            }
            _ => {
                self.badge_anchor.hide();
                None
            }
        };
    }

    /// The panels to draw, with `scene_to_space` taking the scene to the eyes' space.
    pub fn panels(&self, scene_to_space: Mat4) -> impl Iterator<Item = (&Panel, Mat4)> {
        [&self.card, &self.badge]
            .into_iter()
            .zip(self.shown)
            .filter_map(move |(panel, model)| model.map(|model| (panel, scene_to_space * model)))
    }
}

/// Where the indicator goes when neither the controller nor the hand is tracked: low in the view of `head`, a scene transform.
pub fn view_spot(head: Mat4) -> Vec3 {
    head.transform_point3(Vec3::new(0.0, -0.22, -0.55))
}

/// The transcript shows once the right palm has faced within this angle of straight up for [`HOLD`](crate::hand_menu::HOLD), whichever way it leans and wherever the head is, so that any palm-up pose shows it. A palm in the neutral grip faces inwards, 90° from up.
pub const WRIST_SHOW_ANGLE: f32 = 45.0 * std::f32::consts::PI / 180.0;
/// It hides once the palm has turned further than this from straight up, so that it does not flicker at the edge.
pub const WRIST_HIDE_ANGLE: f32 = 55.0 * std::f32::consts::PI / 180.0;

/// The size of the transcript's font pixel on the wrist, and once detached, further away.
pub const WRIST_PIXEL: f32 = 0.001;
pub const DETACHED_PIXEL: f32 = 0.0013;
/// Where a detached transcript stands from the pose its anchor gives: in front of the participant and a little low.
pub const DETACHED_OFFSET: Vec3 = Vec3::new(0.0, -0.1, -0.7);
/// How far the stick must move to count, and how long it is held before it repeats, and how often.
const STICK: f32 = 0.6;
const REPEAT_AFTER: Duration = Duration::from_millis(400);
const REPEAT_EVERY: Duration = Duration::from_millis(120);
/// How firmly the grip is squeezed to count, and how far it is let go before it counts again.
const SQUEEZE: f32 = 0.75;
const RELEASE: f32 = 0.5;

/// The stick as steps: one on moving it, then repeating while it is held.
#[derive(Clone, Debug, Default)]
pub struct Repeat {
    held: Option<(i32, i32)>,
    since: Option<Instant>,
    last: Option<Instant>,
    /// Ignore the stick until it returns to the centre, as when it was already held when the transcript opened.
    blocked: bool,
}

impl Repeat {
    /// A stick that counts only once it has returned to the centre.
    pub fn blocked() -> Self {
        Self {
            blocked: true,
            ..Self::default()
        }
    }

    /// The step for `stick` at `now`, as `(right, down)`, if one is due.
    pub fn update(&mut self, stick: Vec2, now: Instant) -> Option<(i32, i32)> {
        if self.blocked {
            self.blocked = stick.x.abs() > STICK || stick.y.abs() > STICK;
            return None;
        }
        let step = if stick.y.abs() > STICK && stick.y.abs() >= stick.x.abs() {
            (0, if stick.y > 0.0 { -1 } else { 1 })
        } else if stick.x.abs() > STICK {
            (if stick.x > 0.0 { 1 } else { -1 }, 0)
        } else {
            *self = Self::default();
            return None;
        };
        if self.held != Some(step) {
            *self = Self {
                held: Some(step),
                since: Some(now),
                last: Some(now),
                blocked: false,
            };
            return Some(step);
        }
        let since = self.since.unwrap_or(now);
        let last = self.last.unwrap_or(now);
        if now.saturating_duration_since(since) >= REPEAT_AFTER
            && now.saturating_duration_since(last) >= REPEAT_EVERY
        {
            self.last = Some(now);
            return Some(step);
        }
        None
    }
}

/// A firm squeeze of the grip, counted once until it is let go.
#[derive(Clone, Debug, Default)]
pub struct Squeeze {
    held: bool,
}

impl Squeeze {
    /// Whether `value`, the grip's while its action is active, is a new squeeze.
    pub fn update(&mut self, value: Option<f32>) -> bool {
        let value = value.unwrap_or(0.0);
        let pressed = if self.held {
            value > RELEASE
        } else {
            value > SQUEEZE
        };
        let squeezed = pressed && !self.held;
        self.held = pressed;
        squeezed
    }
}

/// What the right wrist's transcript is shown and placed by this frame, in the scene.
#[derive(Clone, Copy, Debug, Default)]
pub struct WristFrame {
    /// The right palm, from hand tracking or the right controller.
    pub palm: Option<Palm>,
    /// Shown from hand tracking alone, without a controller to act with, so it is read-only.
    pub hands_only: bool,
    /// The head's transform, which a detached transcript follows.
    pub head: Option<Mat4>,
    /// The point the transcript faces, which stays put as the head turns: the middle of the head on the headset, from [`head_centre`](crate::hand_menu::head_centre), and the eye in the desktop window, whose head turns about it.
    pub viewer: Option<Vec3>,
    /// Only a space has a conversation.
    pub in_space: bool,
    /// The left hand menu is showing: whichever shows first wins.
    pub menu_shown: bool,
    /// The right trigger is held, attracting or choosing, or the ray highlights a menu item: the palm turns while pointing across the body.
    pub pointing: bool,
    /// The right grip's value, while its action is active.
    pub grip: Option<f32>,
}

/// Whether the right wrist's transcript is shown, on the wrist or detached into the world.
#[derive(Clone, Debug, Default)]
pub struct WristState {
    visibility: Visibility,
    on_hand: bool,
    detached: Option<PanelAnchorState>,
}

/// A detached transcript's anchor.
#[derive(Clone, Debug, Default)]
struct PanelAnchorState(PanelAnchor);

impl WristState {
    /// Whether it takes the right stick, A, B and the grip.
    pub fn active(&self) -> bool {
        self.on_hand || self.detached.is_some()
    }

    pub fn on_hand(&self) -> bool {
        self.on_hand
    }

    pub fn detached(&self) -> bool {
        self.detached.is_some()
    }

    /// Show or hide it by `frame` at `now`. Returns whether it opened on the wrist.
    pub fn show(&mut self, frame: &WristFrame, now: Instant) -> bool {
        if !frame.in_space {
            *self = Self::default();
            return false;
        }
        // While the left menu shows, or the right hand points, it does not show, but it stays if it already shows.
        let blocked = !self.on_hand && (frame.menu_shown || frame.pointing);
        // Only how far the palm is from facing up counts, not where the head is.
        let angle = frame
            .palm
            .filter(|_| !blocked)
            .map(|palm| palm.angle_to_up());
        let shown = self
            .visibility
            .update_within(angle, WRIST_SHOW_ANGLE, WRIST_HIDE_ANGLE, now);
        let was = self.on_hand;
        // Detached, turning the wrist shows no second copy.
        self.on_hand = shown && self.detached.is_none();
        self.on_hand && !was
    }

    /// Detach it where it stands, `centre` facing the participant's `head`, or reattach it to the wrist.
    pub fn toggle_detached(&mut self, centre: Option<Vec3>, head: Option<Vec3>) {
        if self.detached.take().is_some() {
            return;
        }
        if let (Some(centre), Some(head)) = (centre, head) {
            let towards = head - centre;
            let yaw = towards.x.atan2(towards.z);
            let mut anchor = PanelAnchor::default();
            anchor.place(centre, yaw, DETACHED_OFFSET);
            self.detached = Some(PanelAnchorState(anchor));
            self.on_hand = false;
        }
    }

    /// Return it to the wrist, as B does with nothing else to back out of. Returns whether it was detached.
    pub fn reattach(&mut self) -> bool {
        self.detached.take().is_some()
    }
}

/// The right wrist's transcript, with its controls and where it is.
pub struct Wrist {
    panel: Panel,
    state: WristState,
    stick: Repeat,
    squeeze: Squeeze,
    /// Its model in the scene, while shown.
    model: Option<Mat4>,
}

impl Wrist {
    pub fn new(device: &wgpu::Device, renderer: &Renderer) -> Self {
        Self {
            panel: renderer.panel(device, CAPACITY * 4),
            state: WristState::default(),
            stick: Repeat::default(),
            squeeze: Squeeze::default(),
            model: None,
        }
    }

    pub fn state(&self) -> &WristState {
        &self.state
    }

    /// Give the right stick, A and B to the transcript while it is active, at `now`. Returns whether B was taken, so that the space does not also act on it; holding B to leave is unaffected.
    pub fn input(
        &mut self,
        conversation: &mut Conversation,
        stick: Vec2,
        a: bool,
        b: bool,
        now: Instant,
    ) -> bool {
        if !self.state.active() {
            // A stick held as the transcript opens, such as from paging result cards, does not move its focus.
            self.stick = Repeat::blocked();
            return false;
        }
        match self.stick.update(stick, now) {
            Some((0, down)) => conversation.wrist_step(down),
            Some((right, _)) => conversation.wrist_jump(right),
            None => {}
        }
        if a {
            conversation.wrist_accept();
        }
        b && (conversation.wrist_back() || self.state.reattach())
    }

    /// Show, place and draw it for `frame` at `moment`.
    pub fn update(
        &mut self,
        queue: &wgpu::Queue,
        conversation: &mut Conversation,
        frame: WristFrame,
        moment: Moment,
    ) {
        if self.state.show(&frame, moment.now) {
            conversation.wrist_opened();
        }
        // Squeezes count only while it shows.
        if self.squeeze.update(frame.grip) && self.state.active() {
            let centre = self.model.map(|model| model.transform_point3(Vec3::ZERO));
            self.state.toggle_detached(centre, frame.viewer);
        }
        if !self.state.active() {
            self.model = None;
            return;
        }
        let detached = self.state.detached();
        let (columns, pixel) = if detached {
            (DETACHED_COLUMNS, DETACHED_PIXEL)
        } else {
            (WRIST_COLUMNS, WRIST_PIXEL)
        };
        let mut canvas = Canvas::new(columns, pixel);
        canvas.rows = conversation.transcript(columns, frame.hands_only);
        canvas.padding = (6, 5);
        canvas.background = [0.01, 0.02, 0.04, 0.88];
        canvas.border = Some([0.12, 0.85, 1.0, 0.35]);
        self.panel.set(queue, &canvas);
        let (_, height) = canvas.size();
        self.model = match (&mut self.state.detached, frame.head) {
            (Some(anchor), Some(head)) => {
                Some(anchor.0.update(head, moment.dt) * Mat4::from_translation(DETACHED_OFFSET))
            }
            (None, _) => frame
                .palm
                .zip(frame.viewer)
                .map(|(palm, viewer)| on_wrist(&palm, viewer, height)),
            _ => self.model,
        };
    }

    /// The panel to draw, with `scene_to_space` taking the scene to the eyes' space.
    pub fn panels(&self, scene_to_space: Mat4) -> Option<(&Panel, Mat4)> {
        self.model
            .map(|model| (&self.panel, scene_to_space * model))
    }
}

/// The transcript's model on the wrist of `palm`, `height` tall: just above the wrist, offset out of the palm, which faces about up while it shows, on a plane facing `viewer`.
///
/// It is raised above the wrist along its own up, which leans with the way it faces, so `viewer` must stay put as the head turns, as the middle of the head does: facing the eyes, which swing about the neck, it would turn against the head and swing sideways the other way.
pub fn on_wrist(palm: &Palm, viewer: Vec3, height: f32) -> Mat4 {
    let base = palm.wrist + palm.normal * 0.05;
    let normal = (viewer - base).try_normalize().unwrap_or(Vec3::Z);
    let right = Vec3::Y.cross(normal).try_normalize().unwrap_or(Vec3::X);
    let up = normal.cross(right);
    recording::facing(base + up * (height / 2.0 + 0.02), viewer, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::controller;
    use crate::hand_menu::{HOLD, head_centre, head_turned};
    use glam::Quat;

    /// Where the right controller is held, low in front and to the right; the head turns about [`NECK`].
    const GRIP: Vec3 = Vec3::new(0.2, 1.1, -0.35);

    /// The right controller's grip at [`GRIP`], held as in `neutral`, with its -Z forward and its +Y up, turned by `turn`.
    fn grip(turn: Quat) -> controller::Pose {
        controller::Pose {
            position: GRIP.to_array(),
            orientation: turn.to_array(),
        }
    }

    /// Rolling the right hand outwards about the forward axis by 90° turns the palm up.
    fn palm_up() -> Quat {
        Quat::from_rotation_z(-90f32.to_radians())
    }

    /// A frame with the right palm of the controller held as `pose`, and the head turned `yaw` about the neck.
    fn controller_frame(pose: controller::Pose, yaw: f32) -> WristFrame {
        let head = head_turned(NECK, yaw, -20.0);
        WristFrame {
            palm: Palm::from_right_controller(&pose),
            head: Some(head),
            viewer: Some(head_centre(head)),
            in_space: true,
            ..Default::default()
        }
    }

    /// A frame with the right palm up, leaning `angle_degrees` towards the body.
    fn frame(angle_degrees: f32) -> WristFrame {
        let towards_body = Quat::from_rotation_z(angle_degrees.to_radians());
        controller_frame(grip(towards_body * palm_up()), 0.0)
    }

    /// Whether the transcript shows after holding `frame` for longer than the hold, from hidden.
    fn shows(frame: &WristFrame) -> bool {
        let start = Instant::now();
        let mut state = WristState::default();
        state.show(frame, start);
        state.show(frame, start + HOLD);
        state.on_hand()
    }

    #[test]
    fn the_right_palm_faces_inwards_in_a_neutral_grip_and_up_when_rolled_out() {
        // OpenXR's grip +X points into the right palm, so the palm faces along -X: in a neutral grip, to the left, towards the body.
        let neutral = Palm::from_right_controller(&grip(Quat::IDENTITY)).unwrap();
        assert!(neutral.normal.dot(-Vec3::X) > 0.999, "{:?}", neutral.normal);
        assert!((neutral.angle_to_up().to_degrees() - 90.0).abs() < 0.01);
        let up = Palm::from_right_controller(&grip(palm_up())).unwrap();
        assert!(up.normal.dot(Vec3::Y) > 0.999, "{:?}", up.normal);
    }

    #[test]
    fn any_pose_with_the_right_palm_up_shows_it() {
        // A neutral grip, palm inwards, does not show it, however long it is held.
        assert!(!shows(&controller_frame(grip(Quat::IDENTITY), 0.0)));
        // Straight up shows it.
        assert!(shows(&controller_frame(grip(palm_up()), 0.0)));
        // So do 40° towards the body, away from it, forwards and back.
        for tilt in [
            Quat::from_rotation_z(40f32.to_radians()),
            Quat::from_rotation_z(-40f32.to_radians()),
            Quat::from_rotation_x(-40f32.to_radians()),
            Quat::from_rotation_x(40f32.to_radians()),
        ] {
            let frame = controller_frame(grip(tilt * palm_up()), 0.0);
            let angle = frame.palm.unwrap().angle_to_up().to_degrees();
            assert!((angle - 40.0).abs() < 0.01, "{angle}");
            assert!(shows(&frame), "{tilt:?}");
        }
        // Palm down does not.
        assert!(!shows(&controller_frame(
            grip(Quat::from_rotation_z(90f32.to_radians())),
            0.0
        )));
    }

    #[test]
    fn between_the_angles_it_neither_shows_nor_hides() {
        let start = Instant::now();
        // 50° from up does not show it from hidden.
        assert!(!shows(&frame(50.0)));
        // But keeps it shown once it shows.
        let mut state = WristState::default();
        state.show(&frame(0.0), start);
        state.show(&frame(0.0), start + HOLD);
        assert!(state.on_hand());
        state.show(&frame(50.0), start + HOLD * 2);
        state.show(&frame(-50.0), start + HOLD * 3);
        assert!(state.on_hand());
        // 70° hides it.
        state.show(&frame(70.0), start + HOLD * 4);
        assert!(!state.on_hand());
    }

    #[test]
    fn turning_the_head_does_not_change_whether_it_shows() {
        for yaw in [-90.0, -45.0, 0.0, 45.0, 90.0, 180.0] {
            assert!(shows(&controller_frame(grip(palm_up()), yaw)), "{yaw}");
            let tilted = Quat::from_rotation_x(-40f32.to_radians()) * palm_up();
            assert!(shows(&controller_frame(grip(tilted), yaw)), "{yaw}");
            assert!(
                !shows(&controller_frame(grip(Quat::IDENTITY), yaw)),
                "{yaw}"
            );
            let between = Quat::from_rotation_z(50f32.to_radians()) * palm_up();
            assert!(!shows(&controller_frame(grip(between), yaw)), "{yaw}");
        }
    }

    #[test]
    fn with_the_palm_up_it_stands_above_the_wrist_facing_the_head() {
        let frame = controller_frame(grip(palm_up()), 0.0);
        let (palm, viewer) = (frame.palm.unwrap(), frame.viewer.unwrap());
        let height = 0.25;
        let model = on_wrist(&palm, viewer, height);
        let centre = model.transform_point3(Vec3::ZERO);
        // Its lower edge is clear of the hand, and it stays over the wrist.
        let bottom = model.transform_point3(Vec3::new(0.0, -height / 2.0, 0.0));
        assert!(bottom.y > GRIP.y + 0.04, "{bottom}");
        assert!(centre.y > bottom.y + 0.1, "{centre}");
        assert!(
            centre.with_y(0.0).distance(GRIP.with_y(0.0)) < 0.12,
            "{centre}"
        );
        let normal = model.transform_vector3(Vec3::Z).normalize();
        assert!(normal.dot((viewer - centre).normalize()) > 0.999);
    }

    #[test]
    fn the_right_wrist_shows_after_a_hold_and_hides_past_the_outer_angle() {
        let start = Instant::now();
        let hold = HOLD;
        let mut state = WristState::default();
        assert!(!state.show(&frame(20.0), start));
        assert!(state.show(&frame(20.0), start + hold), "opens once");
        assert!(state.on_hand() && state.active());
        assert!(!state.show(&frame(45.0), start + hold * 2));
        assert!(state.on_hand());
        state.show(&frame(60.0), start + hold * 3);
        assert!(!state.on_hand());
        // A hand passing through the pose does not show it.
        state.show(&frame(20.0), start + hold * 4);
        state.show(&frame(60.0), start + hold * 4 + hold / 2);
        state.show(&frame(20.0), start + hold * 5);
        assert!(!state.on_hand());
        // Outside a space it never shows.
        let mut outside = frame(0.0);
        outside.in_space = false;
        let mut state = WristState::default();
        state.show(&outside, start);
        state.show(&outside, start + hold);
        assert!(!state.active());
    }

    #[test]
    fn pointing_or_the_left_menu_keeps_it_from_showing_but_not_from_staying() {
        let start = Instant::now();
        let hold = crate::hand_menu::HOLD;
        for blocked in [
            WristFrame {
                menu_shown: true,
                ..frame(10.0)
            },
            WristFrame {
                pointing: true,
                ..frame(10.0)
            },
        ] {
            let mut state = WristState::default();
            state.show(&blocked, start);
            state.show(&blocked, start + hold * 2);
            assert!(!state.on_hand());
            // Already shown, it stays.
            let mut state = WristState::default();
            state.show(&frame(10.0), start);
            state.show(&frame(10.0), start + hold);
            state.show(&blocked, start + hold * 2);
            assert!(state.on_hand());
        }
    }

    #[test]
    fn the_grip_detaches_it_where_it_stands_and_b_or_the_grip_reattach_it() {
        let start = Instant::now();
        let hold = crate::hand_menu::HOLD;
        let mut state = WristState::default();
        state.show(&frame(10.0), start);
        state.show(&frame(10.0), start + hold);
        let centre = Vec3::new(0.15, 1.3, -0.35);
        let head = Vec3::new(0.0, 1.6, 0.0);
        state.toggle_detached(Some(centre), Some(head));
        assert!(state.detached() && !state.on_hand() && state.active());
        // Turning the wrist away leaves it in the world.
        state.show(&frame(80.0), start + hold * 2);
        assert!(state.active());
        let anchor = &mut state.detached.as_mut().unwrap().0;
        let model = anchor.update(Mat4::from_translation(head), 0.0)
            * Mat4::from_translation(DETACHED_OFFSET);
        assert!(model.transform_point3(Vec3::ZERO).distance(centre) < 1e-4);
        let facing = model.transform_vector3(Vec3::Z);
        assert!(
            facing
                .with_y(0.0)
                .normalize()
                .dot((head - centre).with_y(0.0).normalize())
                > 0.999
        );
        // B with nothing else to back out of returns it to the wrist.
        assert!(state.reattach());
        assert!(!state.active());
        assert!(!state.reattach());
        // So does the grip again.
        state.toggle_detached(Some(centre), Some(head));
        state.toggle_detached(None, None);
        assert!(!state.detached());
        // Leaving the space forgets it.
        state.toggle_detached(Some(centre), Some(head));
        state.show(
            &WristFrame {
                in_space: false,
                ..frame(10.0)
            },
            start,
        );
        assert!(!state.active());
    }

    #[test]
    fn a_firm_squeeze_counts_once_until_let_go() {
        let mut squeeze = Squeeze::default();
        assert!(!squeeze.update(Some(0.6)));
        assert!(squeeze.update(Some(0.8)));
        assert!(!squeeze.update(Some(0.9)));
        assert!(!squeeze.update(Some(0.6)));
        assert!(!squeeze.update(Some(0.4)));
        assert!(squeeze.update(Some(0.8)));
        assert!(!squeeze.update(None));
        assert!(squeeze.update(Some(0.8)));
    }

    #[test]
    fn the_stick_steps_once_then_repeats_while_held() {
        let start = Instant::now();
        let mut stick = Repeat::default();
        let down = Vec2::new(0.0, -0.9);
        assert_eq!(stick.update(down, start), Some((0, 1)));
        assert_eq!(stick.update(down, start + Duration::from_millis(200)), None);
        assert_eq!(
            stick.update(down, start + Duration::from_millis(400)),
            Some((0, 1))
        );
        assert_eq!(stick.update(down, start + Duration::from_millis(450)), None);
        assert_eq!(
            stick.update(down, start + Duration::from_millis(520)),
            Some((0, 1))
        );
        assert_eq!(
            stick.update(Vec2::ZERO, start + Duration::from_millis(600)),
            None
        );
        assert_eq!(
            stick.update(Vec2::new(-0.9, 0.1), start + Duration::from_millis(700)),
            Some((-1, 0))
        );
        assert_eq!(
            stick.update(Vec2::new(0.0, 0.9), start + Duration::from_millis(800)),
            Some((0, -1))
        );
        // Held as the transcript opens, it counts only once it returns to the centre.
        let mut stick = Repeat::blocked();
        assert_eq!(stick.update(down, start), None);
        assert_eq!(stick.update(down, start + Duration::from_secs(1)), None);
        assert_eq!(
            stick.update(Vec2::ZERO, start + Duration::from_secs(2)),
            None
        );
        assert_eq!(
            stick.update(down, start + Duration::from_secs(3)),
            Some((0, 1))
        );
    }

    #[test]
    fn on_the_wrist_it_stands_above_the_wrist_facing_the_head() {
        let palm = Palm {
            wrist: Vec3::new(0.2, 1.1, -0.35),
            normal: Vec3::new(-0.3, 0.6, 0.5).normalize(),
        };
        let head = Vec3::new(0.0, 1.6, 0.0);
        let model = on_wrist(&palm, head, 0.22);
        let centre = model.transform_point3(Vec3::ZERO);
        assert!(centre.y > palm.wrist.y + 0.1);
        let normal = model.transform_vector3(Vec3::Z).normalize();
        assert!(normal.dot((head - centre).normalize()) > 0.999);
    }

    /// A right palm held up below the face and turned towards it, and the neck the head turns about.
    const PALM: Palm = Palm {
        wrist: Vec3::new(0.05, 1.25, -0.3),
        normal: Vec3::new(-0.095, 0.287, 0.953),
    };
    const NECK: Vec3 = Vec3::new(0.0, 1.5, 0.05);
    const HEIGHT: f32 = 0.25;

    /// The transcript's centre and normal on [`PALM`], facing `viewer`.
    fn placed(viewer: Vec3) -> (Vec3, Vec3) {
        let model = on_wrist(&PALM, viewer, HEIGHT);
        (
            model.transform_point3(Vec3::ZERO),
            model.transform_vector3(Vec3::Z).normalize(),
        )
    }

    #[test]
    fn on_the_wrist_it_neither_moves_nor_turns_as_the_head_turns() {
        use crate::hand_menu::{head_centre, head_turned};
        let (centre, normal) = placed(head_centre(head_turned(NECK, 0.0, -30.0)));
        // It stands just above the wrist.
        let base = PALM.wrist + PALM.normal * 0.05;
        assert!(centre.distance(base) < HEIGHT / 2.0 + 0.03);
        assert!(centre.y > base.y);
        for yaw in [-45.0, 0.0, 45.0] {
            for pitch in [-30.0, 0.0] {
                let head = head_turned(NECK, yaw, pitch);
                let (turned, turned_normal) = placed(head_centre(head));
                assert!(turned.distance(centre) < 1e-5, "{yaw} {pitch}: {turned}");
                assert!(turned_normal.dot(normal) > 1.0 - 1e-6, "{yaw} {pitch}");
                // And it faces the eyes, near enough to read.
                let eyes = head.transform_point3(Vec3::ZERO);
                let towards = (eyes - centre).normalize();
                assert!(turned_normal.dot(towards) > 0.9, "{yaw} {pitch}");
            }
        }
    }

    #[test]
    fn turning_the_head_left_never_moves_it_right() {
        use crate::hand_menu::{head_centre, head_turned};
        for direction in [1.0, -1.0] {
            let mut previous = placed(head_centre(head_turned(NECK, 0.0, -30.0))).0;
            for step in 1..=12 {
                // Positive yaw turns the head left, towards -X.
                let yaw = direction * 5.0 * step as f32;
                let centre = placed(head_centre(head_turned(NECK, yaw, -30.0))).0;
                let sideways = (centre.x - previous.x) * direction;
                assert!(sideways <= 1e-5, "{yaw}: {sideways} m");
                previous = centre;
            }
        }
        // Facing the eyes, as it did, it swung several centimetres right as the head turned left.
        let eyes = |yaw: f32| head_turned(NECK, yaw, -30.0).transform_point3(Vec3::ZERO);
        assert!(placed(eyes(45.0)).0.x - placed(eyes(0.0)).0.x > 0.02);
    }

    #[test]
    fn detached_it_stays_put_as_the_head_turns() {
        use crate::hand_menu::{head_centre, head_turned};
        let mut state = WristState::default();
        let head = head_turned(NECK, 0.0, -10.0);
        // Straight ahead, so that turning 40° either way stays within the turn it follows after.
        let at = Vec3::new(0.0, 1.3, -0.45);
        state.toggle_detached(Some(at), Some(head_centre(head)));
        let anchor = &mut state.detached.as_mut().unwrap().0;
        let model = |anchor: &mut PanelAnchor, head: Mat4| {
            anchor.update(head, 0.05) * Mat4::from_translation(DETACHED_OFFSET)
        };
        let start = model(anchor, head);
        assert!(start.transform_point3(Vec3::ZERO).distance(at) < 1e-5);
        for yaw in [-40.0, 40.0, 0.0] {
            for _ in 0..40 {
                assert!(model(anchor, head_turned(NECK, yaw, -10.0)).abs_diff_eq(start, 1e-5));
            }
        }
    }
}
