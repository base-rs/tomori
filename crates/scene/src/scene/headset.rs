// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The headset's own logic, from plain copies of what OpenXR reports: the controllers' actions as buttons, which poses can be trusted, where the scene stands after a recentre, which palm shows what from the controllers and hand tracking, the live microphone's grip, the session's focus and the vibrations to give. `xr.rs` reads the runtime into these types and carries out what they decide.
use std::time::{Duration, Instant};

use glam::{Mat4, Quat, Vec2, Vec3, Vec4};
use tomori_client::engagement::{Focus, Tracker};

use super::{Buttons, Frame, Marker, Senses, Step};
use crate::{
    controller::{self, Pose},
    conversation::Conversation,
    hand_menu::{self, Palm},
    hands,
    log_cadence::Cadence,
    microphone_request::{Answer, MicrophoneRequest},
    pointing::Ray,
    recording::Trackers,
};
use tomori_client::feedback::{Feedback, Hand as Side, Haptic};

/// How often an applied recenter is logged at info, when the runtime repeats them.
const RECENTER_LOG: Duration = Duration::from_secs(60);
/// How far the left grip is squeezed to toggle the live microphone.
const AUDIENCE_GRIP: f32 = 0.75;

/// An action's state, as one sync of the runtime's actions gives it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Action<T> {
    pub value: T,
    /// Bound to a connected controller and usable now.
    pub active: bool,
    /// Changed since the last sync.
    pub changed: bool,
}

/// A single synchronized action snapshot. No pose/tracking state participates in interpreting held buttons, press edges, or inactive/disconnected actions.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Actions {
    pub trigger: Action<f32>,
    pub reset: Action<bool>,
    pub talk: Action<bool>,
    pub back: Action<bool>,
    pub refresh: Action<bool>,
    pub navigation: Action<Vec2>,
    pub presentation_navigation: Action<Vec2>,
    pub sound_toggle: Action<bool>,
    pub replay: Action<bool>,
    pub audience_grip: Action<f32>,
    pub transcript_grip: Action<f32>,
}

/// Where a space is, as the runtime located it, with which parts of it are valid.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Location {
    pub pose: Pose,
    pub position_valid: bool,
    pub orientation_valid: bool,
}

impl Location {
    /// The pose, if both its position and orientation are tracked.
    pub fn tracked(self) -> Option<Pose> {
        (self.position_valid && self.orientation_valid).then_some(self.pose)
    }
}

/// The controllers this frame: buttons and axes from the actions, and poses in the reference space where they are tracked.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Controls {
    /// The right controller's position, while tracked.
    pub controller: Option<Vec3>,
    pub controller_poses: [Option<Pose>; 2],
    /// The right controller's aim pose, while tracked.
    pub aim: Option<Pose>,
    /// The trigger's value, while its action is active; the pointer decides whether a pull attracts or chooses.
    pub trigger: Option<f32>,
    pub reset: bool,
    pub talk: bool,
    pub talk_active: bool,
    pub back: bool,
    pub back_held: bool,
    pub refresh: bool,
    pub navigation: Vec2,
    /// The left stick's action is bound to a connected controller, so that a stick that reads zero because it was lost is not taken for a released one.
    pub navigation_active: bool,
    pub presentation_navigation: Vec2,
    pub sound_toggle: bool,
    pub replay: bool,
    pub audience_grip: f32,
    pub audience_grip_active: bool,
    /// The right grip's value, while its action is active.
    pub transcript_grip: Option<f32>,
}

impl Controls {
    pub fn from_actions(actions: Actions, focused: bool) -> Self {
        if !focused {
            return Self::default();
        }
        let held = |state: Action<bool>| state.active && state.value;
        let pressed = |state: Action<bool>| held(state) && state.changed;
        let axis = |state: Action<Vec2>| {
            if state.active {
                state.value
            } else {
                Vec2::ZERO
            }
        };
        Self {
            trigger: actions.trigger.active.then_some(actions.trigger.value),
            reset: pressed(actions.reset),
            talk: held(actions.talk),
            talk_active: actions.talk.active,
            back: pressed(actions.back),
            back_held: held(actions.back),
            refresh: pressed(actions.refresh),
            navigation: axis(actions.navigation),
            navigation_active: actions.navigation.active,
            presentation_navigation: axis(actions.presentation_navigation),
            sound_toggle: pressed(actions.sound_toggle),
            replay: pressed(actions.replay),
            audience_grip_active: actions.audience_grip.active,
            transcript_grip: actions
                .transcript_grip
                .active
                .then_some(actions.transcript_grip.value),
            audience_grip: if actions.audience_grip.active {
                actions.audience_grip.value
            } else {
                0.0
            },
            ..Self::default()
        }
    }

    /// The grips' locations, left and right, where the runtime could locate them. A missing/invalid pose or a failed optional locate must never discard this frame's button release or press. Do not reuse previous poses.
    pub fn set_locations(&mut self, locations: [Option<Location>; 2]) {
        self.controller = locations[controller::RIGHT]
            .filter(|location| location.position_valid)
            .map(|location| Vec3::from_array(location.pose.position));
        self.controller_poses = locations.map(|location| location.and_then(Location::tracked));
    }

    /// The controller's buttons for the frame step.
    pub fn buttons(&self) -> Buttons {
        Buttons {
            a: self.reset,
            back: self.back,
            back_held: self.back_held,
            y: self.refresh,
            talk: self.talk,
            mute: self.sound_toggle,
            replay: self.replay,
            left_stick: self.navigation,
            right_stick: self.presentation_navigation,
        }
    }
}

/// A held grip toggles once; after focus loss it must be released first.
#[derive(Clone, Copy, Debug, Default)]
pub struct LiveGrip {
    held: bool,
    blocked: bool,
}

impl LiveGrip {
    pub fn blur(&mut self) {
        self.held = false;
        self.blocked = true;
    }

    pub fn update(&mut self, focused: bool, active: bool, pressed: bool) -> bool {
        if !focused || !active {
            self.blur();
            return false;
        }
        if !pressed {
            self.blocked = false;
        }
        let toggle = pressed && !self.held && !self.blocked;
        self.held = pressed;
        toggle
    }
}

/// An eye's field of view, as the tangents' angles to each edge.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Fov {
    pub left: f32,
    pub right: f32,
    pub down: f32,
    pub up: f32,
}

/// An eye's pose in the reference space, as the runtime gives it, and its field of view.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub pose: Pose,
    pub fov: Fov,
}

/// The head between the eyes, in the reference space, facing as the left eye does.
pub fn head_pose(views: &[View; 2]) -> Mat4 {
    let [left, right] = views.map(|view| Vec3::from_array(view.pose.position));
    let [x, y, z, w] = views[0].pose.orientation;
    Mat4::from_rotation_translation(Quat::from_xyzw(x, y, z, w), (left + right) * 0.5)
}

/// An eye's view and projection: its asymmetric frustum, right-handed with -Z forward and WebGPU's depth from 0 to 1. wgpu's Vulkan backend handles the framebuffer Y convention itself.
pub fn view_projection(view: &View) -> Mat4 {
    let fov = view.fov;
    let (left, right, down, up) = (
        fov.left.tan(),
        fov.right.tan(),
        fov.down.tan(),
        fov.up.tan(),
    );
    let (near, far) = (0.05, 50.0);
    let projection = Mat4::from_cols(
        Vec4::new(2.0 / (right - left), 0.0, 0.0, 0.0),
        Vec4::new(0.0, 2.0 / (up - down), 0.0, 0.0),
        Vec4::new(
            (right + left) / (right - left),
            (up + down) / (up - down),
            -far / (far - near),
            -1.0,
        ),
        Vec4::new(0.0, 0.0, -(far * near) / (far - near), 0.0),
    );
    let [x, y, z, w] = view.pose.orientation;
    projection
        * Mat4::from_rotation_translation(
            Quat::from_xyzw(x, y, z, w),
            Vec3::from_array(view.pose.position),
        )
        .inverse()
}

/// Where the scene starts: under the head, turned to face where it looks, level whatever its pitch. Looking straight up or down has no well-defined heading, so it waits for one rather than generating an unstable yaw.
pub fn initial_scene_transform(views: &[View; 2]) -> Option<Mat4> {
    let head = head_pose(views);
    let forward = head.transform_vector3(-Vec3::Z);
    let horizontal = Vec3::new(forward.x, 0.0, forward.z);
    if !horizontal.is_finite() || horizontal.length_squared() < 0.0001 {
        return None;
    }
    let yaw = (-horizontal.x).atan2(-horizontal.z);
    let eye = head.transform_point3(Vec3::ZERO);
    if !eye.is_finite() {
        return None;
    }
    Some(Mat4::from_rotation_translation(
        Quat::from_rotation_y(yaw),
        Vec3::new(eye.x, 0.0, eye.z),
    ))
}

/// Positions the simulation once at startup, then follows explicit runtime reference-space changes only at their effective predicted display time, in nanoseconds. It does not re-anchor on focus/visibility changes or normal head movement. Travel, the participant's rig, then takes the room it places into the space's scene, and a recentre keeps it; see `design/locomotion.md`.
#[derive(Debug)]
pub struct SceneAnchor {
    scene_to_space: Option<Mat4>,
    /// The rig: from the room to the space's scene.
    travel: Mat4,
    pending: Vec<i64>,
    recenter_log: Cadence,
}

impl Default for SceneAnchor {
    fn default() -> Self {
        Self {
            scene_to_space: None,
            travel: Mat4::IDENTITY,
            pending: Vec::new(),
            recenter_log: Cadence::new(RECENTER_LOG),
        }
    }
}

impl SceneAnchor {
    pub fn schedule(&mut self, time: i64) {
        self.pending.push(time);
    }

    /// Follow the eyes' `views` for display at `display_time`, applying any recentre due by then, at `now`.
    pub fn update(&mut self, views: &[View; 2], display_time: i64, now: Instant) {
        let recentered = self.pending.iter().any(|time| *time <= display_time);
        self.pending.retain(|time| *time > display_time);
        if recentered && self.scene_to_space.is_some() {
            // A system recenter makes the new local origin face the user's current forward direction. Reset the startup offset so the existing swarm appears in front, without resetting its state.
            self.scene_to_space = Some(Mat4::IDENTITY);
            // The runtime can repeat reference-space changes; log at most one a minute at info.
            match self.recenter_log.record(now) {
                Some(due) if due.skipped > 0 => log::info!(
                    "Applied OpenXR recenter at display time {display_time} ns, and {} more since the last logged",
                    due.skipped
                ),
                Some(_) => log::info!("Applied OpenXR recenter at display time {display_time} ns"),
                None => log::debug!("Applied OpenXR recenter at display time {display_time} ns"),
            }
        } else if self.scene_to_space.is_none() {
            self.scene_to_space = initial_scene_transform(views);
            if self.scene_to_space.is_some() {
                log::info!("Aligned scene to initial headset direction");
            }
        }
    }

    /// Place the room in the space's scene with the rig's `travel`.
    pub fn set_travel(&mut self, travel: Mat4) {
        self.travel = travel;
    }

    /// The transform from the space's scene to the reference space, with travel.
    pub fn transform(&self) -> Mat4 {
        self.scene_to_space.unwrap_or(Mat4::IDENTITY) * self.travel.inverse()
    }

    pub fn to_scene(&self, point: Vec3) -> Vec3 {
        self.transform().inverse().transform_point3(point)
    }

    /// A pose in the reference space, as a validated pose in the scene.
    pub fn to_scene_pose(&self, pose: Pose) -> Option<Pose> {
        pose.valid()
            .then(|| self.transform().inverse() * pose.transform())
            .and_then(Pose::from_transform)
    }

    /// A reference-space transform, such as the head, as a validated pose in the space's scene coordinates.
    pub fn to_scene_transform(&self, transform: Mat4) -> Option<Pose> {
        let reference = Pose::from_transform(transform)?;
        Pose::from_transform(self.transform().inverse() * reference.transform())
    }
}

/// A tracked hand joint, as the runtime located it in the reference space.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Joint {
    pub position: Vec3,
    pub radius: f32,
    pub position_valid: bool,
}

/// A tracked hand in the scene, from its joints in the reference space: invalid joints are hidden, a radius the runtime does not give is guessed, and a hand without a single bone is not drawn.
pub fn hand(joints: &[Joint; hands::JOINT_COUNT], scene_from_space: Mat4) -> Option<hands::Hand> {
    let mut out = hands::Hand {
        joints: [None; hands::JOINT_COUNT],
    };
    for (slot, joint) in out.joints.iter_mut().zip(joints) {
        if !joint.position_valid {
            continue;
        }
        let radius = if joint.radius.is_finite() && joint.radius > 0.0 {
            joint.radius.clamp(0.001, 0.1)
        } else {
            0.006
        };
        let value = hands::Joint {
            position: scene_from_space.transform_point3(joint.position).to_array(),
            radius,
        };
        if value.valid() {
            *slot = Some(value);
        }
    }
    hands::BONES
        .iter()
        .any(|(a, b)| out.joints[*a].is_some() && out.joints[*b].is_some())
        .then_some(out)
}

/// The OpenXR session's state, as far as the headset cares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionState {
    Ready,
    Visible,
    Focused,
    Stopping,
    Exiting,
    LossPending,
    /// Idle, synchronized, or one the headset does not act on.
    Other,
}

/// What the session should do after a state change: begin, end or leave it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lifecycle {
    Begin,
    End,
    Exit,
    Continue,
}

/// What a headset frame draws and reports.
pub struct Drawn<'a> {
    pub frame: Frame<'a>,
    /// The controllers and hands to draw, in the scene.
    pub controllers: [Option<Pose>; 2],
    pub hands: [Option<hands::Hand>; 2],
    /// Takes the scene to the eyes' reference space.
    pub scene_to_space: Mat4,
    /// Each eye's view and projection.
    pub projections: [Mat4; 2],
}

/// The headset's logic between frames.
pub struct Headset {
    step: Step,
    anchor: SceneAnchor,
    /// Whether the headset is worn and Tomori has input, reported to the server; see `design/spaces.md`.
    engagement: Tracker,
    audience_toggle: LiveGrip,
    focused: bool,
    running: bool,
    /// Ask for the microphone once the scene has been shown with focus, instead of on the first recording; see [`MicrophoneRequest`].
    microphone: MicrophoneRequest,
    last_frame: Instant,
}

impl Headset {
    /// Start, with the conversation decided on the right wrist and the controllers vibrating as things change. See `design/wrist-transcript.md` and `design/interaction.md`.
    pub fn new(conversation: &mut Conversation, feedback: &mut Feedback, now: Instant) -> Self {
        conversation.decide_on_wrist();
        let engagement = Tracker::new();
        conversation.set_engagement(engagement.engagement());
        feedback.set_haptics(true);
        Self {
            step: Step::default(),
            anchor: SceneAnchor::default(),
            engagement,
            audience_toggle: LiveGrip::default(),
            focused: false,
            running: false,
            microphone: MicrophoneRequest::default(),
            last_frame: now,
        }
    }

    /// Whether the session has focus, and so input.
    pub fn focused(&self) -> bool {
        self.focused
    }

    /// Whether the session is running, between beginning and ending it.
    pub fn running(&self) -> bool {
        self.running
    }

    fn blur(&mut self, conversation: &mut Conversation) {
        self.audience_toggle.blur();
        self.step.blur(conversation);
    }

    /// The session changed to `state` at `now`. The microphone is not asked for here, before the scene has been shown; see [`Self::microphone_due`].
    pub fn session(
        &mut self,
        state: SessionState,
        conversation: &mut Conversation,
        now: Instant,
    ) -> Lifecycle {
        // Do not lose a real focus-loss transition if the runtime also queues a refocus in this poll batch.
        if self.focused && state != SessionState::Focused {
            self.blur(conversation);
            self.microphone.blur();
        }
        self.focused = state == SessionState::Focused;
        let focus = match state {
            SessionState::Focused => Focus::Focused,
            SessionState::Visible => Focus::Visible,
            _ => Focus::Hidden,
        };
        if let Some(changed) = self.engagement.set_focus(focus) {
            log::info!("Engagement: {changed:?}, as the session is {state:?}");
            conversation.set_engagement(changed);
        }
        match state {
            SessionState::Ready => {
                self.running = true;
                self.last_frame = now;
                Lifecycle::Begin
            }
            SessionState::Stopping => {
                self.running = false;
                Lifecycle::End
            }
            SessionState::Exiting | SessionState::LossPending => Lifecycle::Exit,
            _ => Lifecycle::Continue,
        }
    }

    /// After each turn of the frame loop at `now`, whether the scene was `presented` in a submitted frame: whether to ask the platform for the microphone now, with [`Self::microphone_answered`] given its answer. It is asked once the scene has been shown with focus for a moment, and then checked again until answered; the frame loop never waits for it. See `design/arrival.md`.
    pub fn microphone_due(&mut self, presented: bool, now: Instant) -> bool {
        self.microphone
            .step(self.focused && self.running, presented, now)
    }

    /// The platform said at `now` whether the microphone is `allowed`. While the request is unanswered, the panel says so.
    pub fn microphone_answered(
        &mut self,
        allowed: Result<bool, String>,
        conversation: &mut Conversation,
        now: Instant,
    ) {
        match self.microphone.answered(allowed, now) {
            Answer::Asked => log::info!("Asked for microphone access"),
            Answer::Waiting => {}
            Answer::Allowed => log::info!("Microphone access is allowed"),
            Answer::Declined => {
                log::info!("Microphone access was not allowed; the first recording asks again")
            }
            Answer::Failed(error) => log::warn!("{error}"),
        }
        conversation.microphone_request(self.microphone.waiting());
    }

    /// The proximity sensor says whether the headset is worn. Meta's runtime does not always deliver "removed", so the session's state is followed too.
    pub fn worn(&mut self, worn: bool, conversation: &mut Conversation) {
        let change = if worn { "put on" } else { "taken off" };
        log::info!("Headset {change}");
        if let Some(changed) = self.engagement.set_worn(worn) {
            log::info!("Engagement: {changed:?}");
            conversation.set_engagement(changed);
        }
    }

    /// The runtime will recentre the reference space at display time `time`, in nanoseconds, which also places the panel in front of the participant again.
    pub fn recentre(&mut self, time: i64) {
        self.anchor.schedule(time);
        self.step.recentre();
    }

    /// Before each frame: without focus, or while the session is not running, nothing held carries over.
    pub fn idle(&mut self, conversation: &mut Conversation) {
        if !self.focused || !self.running {
            self.blur(conversation);
        }
    }

    /// A frame begins at `now`. Returns the seconds since the last.
    pub fn tick(&mut self, now: Instant) -> f32 {
        let dt = now.duration_since(self.last_frame).as_secs_f32();
        self.last_frame = now;
        dt
    }

    /// Act on this frame's `controls` at `now`. Button and axis actions run on every running frame, even if the compositor or head and controller tracking cannot render. Only actual session focus loss cancels an utterance; an X release while still focused must finalize and send it instead.
    pub fn buttons(
        &mut self,
        controls: &Controls,
        conversation: &mut Conversation,
        feedback: &mut Feedback,
        now: Instant,
    ) {
        if self.audience_toggle.update(
            self.focused,
            controls.audience_grip_active,
            controls.audience_grip > AUDIENCE_GRIP,
        ) {
            conversation.toggle_live();
        }
        self.step.buttons(
            &controls.buttons(),
            self.focused,
            conversation,
            feedback,
            now,
        );
        // A left stick that was lost reads as released, which must not land a teleport being aimed; see `design/locomotion.md`.
        if !controls.navigation_active {
            self.step.cancel_travel();
        }
    }

    /// Move the scene on by `dt` seconds to `now`, for display at `display_time`, in nanoseconds, from the eyes' `views`, the `controls` with their poses, and the tracked `hands`' joints, all in the reference space.
    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        views: &[View; 2],
        display_time: i64,
        controls: &Controls,
        hands: [Option<[Joint; hands::JOINT_COUNT]>; 2],
        conversation: &mut Conversation,
        feedback: &mut Feedback,
        now: Instant,
        dt: f32,
    ) -> Drawn<'_> {
        let anchor = &mut self.anchor;
        anchor.update(views, display_time, now);
        // Everything below is in the space's scene, where the participant has travelled to.
        anchor.set_travel(self.step.rig(conversation).matrix());
        // The simulation stays in its own coordinates; tracked controller positions must be transformed into that same space after recenter.
        let controller = controls
            .controller
            .map(|position| anchor.to_scene(position));
        let controllers = controls
            .controller_poses
            .map(|pose| pose.and_then(|pose| anchor.to_scene_pose(pose)));
        let scene_from_space = anchor.transform().inverse();
        let tracked = hands.map(|joints| joints.and_then(|joints| hand(&joints, scene_from_space)));
        // Outside a space, arrival's objects are pointed at with the right controller's aim ray. A trigger pull that begins on one of them chooses it; any other pull attracts the swarm, as it does inside a space, where there is nothing to point at.
        let ray = controls
            .aim
            .and_then(|pose| anchor.to_scene_pose(pose))
            .and_then(|pose| {
                Ray::from_pose(
                    Vec3::from_array(pose.position),
                    Quat::from_array(pose.orientation),
                )
            });
        let space_head = head_pose(views);
        let head_pose = anchor.to_scene_transform(space_head);
        let scene_head = head_pose.map(|pose| pose.transform());
        // What faces the head faces the middle of it, which stays put as the head turns, rather than the eyes, which swing about the neck.
        let head = scene_head.map(hand_menu::head_centre);
        let trackers = Trackers {
            grip: controllers[controller::LEFT],
            wrist: tracked[controller::LEFT]
                .as_ref()
                .and_then(|hand| hand.joints[1])
                .map(|joint| Vec3::from_array(joint.position)),
            view: scene_head.map(crate::controller_ui::view_spot),
        };
        // The hand menu shows above the left wrist while the left palm is turned up, from hand tracking or else the left controller. Pinned with Y, it stands above the left controller facing the head, whatever its angle.
        let left_palm = if self.step.pinned() {
            trackers
                .target(Vec3::new(0.0, 0.03, 0.0))
                .zip(head)
                .and_then(|(at, head)| {
                    Some(Palm {
                        wrist: at,
                        normal: (head - at).try_normalize()?,
                    })
                })
        } else {
            tracked[controller::LEFT]
                .as_ref()
                .and_then(|hand| Palm::from_hand(Side::Left, hand))
                .or_else(|| {
                    controllers[controller::LEFT]
                        .as_ref()
                        .and_then(|pose| Palm::from_controller(Side::Left, pose))
                })
        };
        // The right palm shows the transcript, from the right controller, or else hand tracking, which only reads.
        let right_controller = controllers[controller::RIGHT]
            .as_ref()
            .and_then(|pose| Palm::from_controller(Side::Right, pose));
        let right_hand = tracked[controller::RIGHT]
            .as_ref()
            .and_then(|hand| Palm::from_hand(Side::Right, hand));
        let senses = Senses {
            focused: self.focused,
            head: scene_head,
            head_pose,
            viewer: head,
            space_head,
            ray,
            trigger: controls.trigger,
            pointing: self.step.pointing(controls.trigger),
            left_palm,
            right_palm: right_controller.or(right_hand),
            hands_only: right_controller.is_none(),
            grip: controls.transcript_grip,
            trackers,
            attractor: controller,
            // The attractor is drawn where the right controller is, unless its model or the right hand is.
            marker: if controllers[controller::RIGHT].is_some()
                || tracked[controller::RIGHT].is_some()
            {
                Marker::Hidden
            } else {
                Marker::Always
            },
            controllers,
            hands: tracked,
            cues: true,
            beam: true,
        };
        let scene_to_space = anchor.transform();
        let frame = self.step.update(&senses, conversation, feedback, now, dt);
        Drawn {
            frame,
            controllers,
            hands: tracked,
            scene_to_space,
            projections: views.map(|view| view_projection(&view)),
        }
    }

    /// The vibrations due by `now`. The runtime ignores vibrations while another application has focus, so they are dropped rather than felt late.
    pub fn haptics(&self, feedback: &mut Feedback, now: Instant) -> Vec<(Side, Haptic)> {
        let due = feedback.haptics(now);
        if self.focused { due } else { Vec::new() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{hand_menu::HOLD, scene::fixtures::*};
    use std::time::Duration;
    use tomori_client::feedback::{Hand as Side, Settings};
    use tomori_protocol::{ConnectionRole, Engagement};

    const FRAME: f32 = 1.0 / 72.0;

    fn action<T>(value: T, active: bool, changed: bool) -> Action<T> {
        Action {
            value,
            active,
            changed,
        }
    }

    fn actions(held: bool, active: bool, changed: bool) -> Actions {
        Actions {
            trigger: action(if held { 1.0 } else { 0.0 }, active, changed),
            reset: action(held, active, changed),
            talk: action(held, active, changed),
            back: action(held, active, changed),
            refresh: action(held, active, changed),
            navigation: action(Vec2::new(0.8, -0.7), active, changed),
            presentation_navigation: action(Vec2::new(-0.9, 0.6), active, changed),
            sound_toggle: action(held, active, changed),
            replay: action(held, active, changed),
            audience_grip: action(if held { 1.0 } else { 0.0 }, active, changed),
            transcript_grip: action(if held { 1.0 } else { 0.0 }, active, changed),
        }
    }

    const IDENTITY: Pose = Pose {
        position: [0.0, 0.0, 0.0],
        orientation: [0.0, 0.0, 0.0, 1.0],
    };

    fn location(position_valid: bool, orientation_valid: bool) -> Option<Location> {
        Some(Location {
            pose: IDENTITY,
            position_valid,
            orientation_valid,
        })
    }

    fn assert_press_edges(controls: &Controls, expected: bool) {
        assert_eq!(controls.reset, expected);
        assert_eq!(controls.back, expected);
        assert_eq!(controls.refresh, expected);
        assert_eq!(controls.sound_toggle, expected);
        assert_eq!(controls.replay, expected);
    }

    #[test]
    fn held_x_survives_tracking_loss_and_releases_while_untracked() {
        let frames = [
            (true, true, [location(true, true), location(true, true)]),
            // The runtime can supply invalid locations, no active pose, or a locate error without losing the independent X action.
            (true, false, [location(false, false), None]),
            (false, true, [None, None]),
        ];
        for (held, changed, locations) in frames {
            let mut controls = Controls::from_actions(actions(held, true, changed), true);
            controls.set_locations(locations);
            assert_eq!(controls.talk, held);
            assert_eq!(controls.audience_grip, if held { 1.0 } else { 0.0 });
            assert_press_edges(&controls, held && changed);
        }
    }

    #[test]
    fn new_untracked_presses_and_menu_axes_do_not_require_poses() {
        for locations in [
            [None, None],
            [location(false, false), location(false, false)],
        ] {
            let mut controls = Controls::from_actions(actions(true, true, true), true);
            controls.set_locations(locations);
            assert!(controls.talk && controls.trigger == Some(1.0));
            assert_press_edges(&controls, true);
            assert_eq!(controls.audience_grip, 1.0);
            assert_eq!(controls.transcript_grip, Some(1.0));
            assert_eq!(controls.navigation.x, 0.8);
            assert!(controls.navigation_active);
            assert_eq!(controls.presentation_navigation.x, -0.9);
            assert!(controls.controller.is_none());
            assert!(controls.controller_poses.iter().all(Option::is_none));
        }
    }

    #[test]
    fn pose_validity_only_changes_spatial_controls_and_never_reuses_old_poses() {
        let mut controls = Controls::from_actions(actions(true, true, true), true);
        controls.set_locations([location(true, true), location(true, true)]);
        assert!(controls.controller_poses.iter().all(Option::is_some));
        controls.set_locations([location(false, true), location(true, false)]);
        assert!(controls.controller.is_some());
        assert!(controls.controller_poses.iter().all(Option::is_none));
        controls.set_locations([None, None]);
        assert!(controls.controller.is_none());
        assert!(controls.controller_poses.iter().all(Option::is_none));
        assert!(controls.talk);
        assert_press_edges(&controls, true);
    }

    #[test]
    fn one_hand_without_a_location_does_not_hide_the_other_controller() {
        let mut controls = Controls::from_actions(actions(true, true, true), true);
        controls.set_locations([None, location(true, true)]);
        assert!(controls.controller_poses[controller::LEFT].is_none());
        assert!(controls.controller_poses[controller::RIGHT].is_some());
        assert!(controls.controller.is_some());
        assert!(controls.talk);
        assert_press_edges(&controls, true);
    }

    #[test]
    fn inactive_actions_and_focus_loss_clear_held_buttons_axes_and_edges() {
        for (active, focused) in [(false, true), (true, false), (false, false)] {
            // Deliberately stale held/changed values: neither a disconnected controller nor an unfocused session may keep recording alive.
            let controls = Controls::from_actions(actions(true, active, true), focused);
            assert!(!controls.talk && controls.trigger.is_none());
            assert!(!controls.talk_active);
            assert_press_edges(&controls, false);
            assert_eq!(controls.audience_grip, 0.0);
            assert_eq!(controls.transcript_grip, None);
            assert_eq!(controls.navigation, Vec2::ZERO);
            assert!(!controls.navigation_active);
            assert_eq!(controls.presentation_navigation, Vec2::ZERO);
        }
        // Refocusing samples fresh action state, not a cached button. Held X is level-triggered; one-shot menu actions still require a new edge.
        let controls = Controls::from_actions(actions(true, true, false), true);
        assert!(controls.talk);
        assert_press_edges(&controls, false);
        let released = Controls::from_actions(actions(false, true, true), true);
        assert!(!released.talk);
        assert_press_edges(&released, false);
    }

    #[test]
    fn audience_grip_does_not_start_push_to_talk() {
        let mut input = actions(false, true, false);
        input.audience_grip = action(1.0, true, true);
        let controls = Controls::from_actions(input, true);
        assert_eq!(controls.audience_grip, 1.0);
        assert!(!controls.talk);
        assert_press_edges(&controls, false);
    }

    #[test]
    fn the_controllers_buttons_are_the_steps_buttons() {
        let controls = Controls::from_actions(actions(true, true, true), true);
        assert_eq!(
            controls.buttons(),
            Buttons {
                a: true,
                back: true,
                back_held: true,
                y: true,
                talk: true,
                mute: true,
                replay: true,
                left_stick: Vec2::new(0.8, -0.7),
                right_stick: Vec2::new(-0.9, 0.6),
            }
        );
    }

    #[test]
    fn live_grip_requires_release_after_focus_loss() {
        let mut grip = LiveGrip::default();
        assert!(grip.update(true, true, true));
        assert!(!grip.update(true, true, true));
        assert!(!grip.update(false, true, true));
        assert!(!grip.update(true, true, true));
        assert!(!grip.update(true, true, false));
        assert!(grip.update(true, true, true));
        // An inactive action is not evidence that the grip was released.
        assert!(!grip.update(true, false, false));
        assert!(!grip.update(true, true, true));
        assert!(!grip.update(true, true, false));
        assert!(grip.update(true, true, true));
    }

    #[test]
    fn controller_pose_preserves_position_and_orientation_through_scene_anchor() {
        let scene_to_space =
            Mat4::from_rotation_translation(Quat::from_rotation_y(0.7), Vec3::new(1.0, 0.0, -2.0));
        let anchor = SceneAnchor {
            scene_to_space: Some(scene_to_space),
            ..SceneAnchor::default()
        };
        let q = Quat::from_rotation_x(0.4) * Quat::from_rotation_y(-0.2);
        let pose = Pose {
            position: [0.3, 1.2, -0.8],
            orientation: q.to_array(),
        };
        let scene_pose = anchor.to_scene_pose(pose).unwrap();
        let expected = Mat4::from_rotation_translation(q, Vec3::new(0.3, 1.2, -0.8));
        assert!((scene_to_space * scene_pose.transform()).abs_diff_eq(expected, 1e-5));
        let invalid = Pose {
            orientation: [0.0; 4],
            ..pose
        };
        assert!(anchor.to_scene_pose(invalid).is_none());
        assert!(
            anchor
                .to_scene_transform(Mat4::from_scale(Vec3::NAN))
                .is_none()
        );
    }

    const FOV: Fov = Fov {
        left: -0.7,
        right: 0.7,
        down: -0.7,
        up: 0.7,
    };

    fn stereo(position: Vec3, orientation: Quat) -> [View; 2] {
        [-0.032, 0.032].map(|x| View {
            pose: Pose {
                position: (position + orientation * Vec3::X * x).to_array(),
                orientation: orientation.to_array(),
            },
            fov: FOV,
        })
    }

    #[test]
    fn startup_scene_aligns_with_head_yaw_and_position_but_not_pitch() {
        for yaw in [0.0, 0.7, -1.5, std::f32::consts::PI] {
            let orientation = Quat::from_rotation_y(yaw) * Quat::from_rotation_x(0.35);
            let views = stereo(Vec3::new(2.0, 1.7, -3.0), orientation);
            let transform = initial_scene_transform(&views).unwrap();
            let scene_forward = transform.transform_vector3(-Vec3::Z);
            let expected = Quat::from_rotation_y(yaw) * -Vec3::Z;
            assert!(scene_forward.distance(expected) < 0.0001);
            assert!(
                transform
                    .transform_point3(Vec3::ZERO)
                    .distance(Vec3::new(2.0, 0.0, -3.0))
                    < 0.0001
            );
            assert!(
                (transform.transform_point3(crate::swarm::CENTER).y - crate::swarm::CENTER.y).abs()
                    < 0.0001
            );
        }
        assert!(
            initial_scene_transform(&stereo(
                Vec3::Y,
                Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)
            ))
            .is_none()
        );
        assert!(
            initial_scene_transform(&stereo(Vec3::splat(f32::INFINITY), Quat::IDENTITY)).is_none()
        );
    }

    #[test]
    fn recenter_waits_until_runtime_change_time_and_preserves_simulation_coordinates() {
        let mut anchor = SceneAnchor::default();
        let views = stereo(Vec3::new(2.0, 1.6, 1.0), Quat::from_rotation_y(1.0));
        let now = Instant::now();
        anchor.update(&views, 10, now);
        let original = anchor.transform();
        let controller = Vec3::new(0.2, 1.0, -0.4);
        assert!(
            anchor
                .to_scene(original.transform_point3(controller))
                .distance(controller)
                < 0.0001
        );
        anchor.schedule(40);
        anchor.schedule(20);
        anchor.update(&views, 19, now);
        assert_eq!(anchor.transform(), original);
        anchor.update(&views, 20, now);
        assert_eq!(anchor.transform(), Mat4::IDENTITY);
        assert_eq!(anchor.pending.len(), 1);
        anchor.update(&views, 40, now);
        assert!(anchor.pending.is_empty());
        assert_eq!(anchor.to_scene(controller), controller);
        // Repeated recentres are logged at most once a minute, counting those skipped.
        for (time, at) in [(50, 1), (60, 61)] {
            anchor.schedule(time);
            anchor.update(&views, time, now + Duration::from_secs(at));
            assert_eq!(anchor.transform(), Mat4::IDENTITY);
        }
        // Before the scene is aligned, a recentre waits for a heading.
        let mut unaligned = SceneAnchor::default();
        unaligned.schedule(5);
        let upwards = stereo(Vec3::Y, Quat::from_rotation_x(std::f32::consts::FRAC_PI_2));
        unaligned.update(&upwards, 10, now);
        assert_eq!(unaligned.transform(), Mat4::IDENTITY);
        assert!(unaligned.scene_to_space.is_none());
    }

    #[test]
    fn the_head_is_between_the_eyes() {
        let views = stereo(Vec3::new(0.0, 1.6, 0.4), Quat::IDENTITY);
        assert!(
            head_pose(&views)
                .transform_point3(Vec3::ZERO)
                .distance(Vec3::new(0.0, 1.6, 0.4))
                < 0.0001
        );
    }

    #[test]
    fn xr_projection_maps_frustum_and_depth_correctly() {
        let view = View {
            pose: IDENTITY,
            fov: Fov {
                left: -0.7,
                right: 0.9,
                down: -0.8,
                up: 0.75,
            },
        };
        let m = view_projection(&view);
        for (x, y, z, expected) in [
            (view.fov.left.tan(), 0.0, -1.0, -1.0),
            (view.fov.right.tan(), 0.0, -1.0, 1.0),
        ] {
            assert!((m.project_point3(Vec3::new(x, y, z)).x - expected).abs() < 0.0001);
        }
        assert!(m.project_point3(Vec3::new(0.0, 0.0, -0.05)).z.abs() < 0.0001);
        assert!((m.project_point3(Vec3::new(0.0, 0.0, -50.0)).z - 1.0).abs() < 0.0001);
    }

    #[test]
    fn hand_joints_are_hidden_when_invalid_and_moved_into_the_scene() {
        let scene_from_space = Mat4::from_translation(Vec3::new(1.0, 0.0, 0.0));
        let mut joints = [Joint::default(); hands::JOINT_COUNT];
        assert!(
            hand(&joints, scene_from_space).is_none(),
            "no joints, no hand"
        );
        for (index, joint) in joints.iter_mut().enumerate() {
            *joint = Joint {
                position: Vec3::new(0.0, 1.0, -0.01 * index as f32),
                radius: 0.01,
                position_valid: true,
            };
        }
        joints[2].radius = f32::NAN;
        joints[3].radius = 1.0;
        joints[4].position_valid = false;
        joints[5].position = Vec3::splat(f32::INFINITY);
        let tracked = hand(&joints, scene_from_space).expect("a hand");
        let joint = tracked.joints[1].unwrap();
        assert_eq!(joint.position, [1.0, 1.0, -0.01]);
        assert_eq!(joint.radius, 0.01);
        assert_eq!(tracked.joints[2].unwrap().radius, 0.006, "guessed");
        assert_eq!(tracked.joints[3].unwrap().radius, 0.1, "clamped");
        assert!(tracked.joints[4].is_none() && tracked.joints[5].is_none());
    }

    #[test]
    fn a_location_is_tracked_only_with_both_position_and_orientation() {
        assert_eq!(location(true, true).unwrap().tracked(), Some(IDENTITY));
        assert_eq!(location(true, false).unwrap().tracked(), None);
        assert_eq!(location(false, true).unwrap().tracked(), None);
    }

    #[test]
    fn the_session_begins_ends_and_reports_focus() {
        let mut conversation = joined();
        let mut feedback = felt();
        let start = Instant::now();
        let mut headset = Headset::new(&mut conversation, &mut feedback, start);
        assert!(!headset.focused() && !headset.running());
        let ready = headset.session(SessionState::Ready, &mut conversation, start);
        assert_eq!(ready, Lifecycle::Begin);
        assert!(headset.running());
        let visible = headset.session(SessionState::Visible, &mut conversation, start);
        assert_eq!(visible, Lifecycle::Continue);
        headset.session(SessionState::Focused, &mut conversation, start);
        assert!(headset.focused());
        assert_eq!(headset.engagement.engagement(), Engagement::Engaged);
        // Taken off and put on again.
        headset.worn(false, &mut conversation);
        assert_eq!(headset.engagement.engagement(), Engagement::Away);
        headset.worn(true, &mut conversation);
        assert_eq!(headset.engagement.engagement(), Engagement::Engaged);
        // Focus lost while recording cancels it.
        headset.buttons(
            &Controls {
                talk: true,
                ..Controls::default()
            },
            &mut conversation,
            &mut feedback,
            start,
        );
        assert!(conversation.recording().is_some());
        headset.session(SessionState::Visible, &mut conversation, start);
        assert!(!headset.focused());
        assert!(conversation.recording().is_none());
        headset.session(SessionState::Focused, &mut conversation, start);
        assert_eq!(
            headset.session(SessionState::Stopping, &mut conversation, start),
            Lifecycle::End
        );
        assert!(!headset.running());
        for state in [SessionState::Exiting, SessionState::LossPending] {
            assert_eq!(
                headset.session(state, &mut conversation, start),
                Lifecycle::Exit
            );
        }
        assert_eq!(
            headset.session(SessionState::Other, &mut conversation, start),
            Lifecycle::Continue
        );
        // Taken off, the participant is away.
        headset.worn(false, &mut conversation);
        assert_eq!(headset.engagement.engagement(), Engagement::Away);
        headset.worn(false, &mut conversation);
    }

    #[test]
    fn the_microphone_is_asked_for_once_the_scene_is_shown_and_followed_without_waiting() {
        use crate::conversation::MICROPHONE_REQUEST;
        use crate::microphone_request::{CHECK, RETURNED, SETTLE};

        let mut conversation = joined();
        let mut feedback = felt();
        let start = Instant::now();
        let mut headset = Headset::new(&mut conversation, &mut feedback, start);
        // Not running, or running without focus, it waits however long.
        assert!(!headset.microphone_due(true, start));
        assert!(!headset.microphone_due(true, start + SETTLE * 3));
        headset.session(SessionState::Ready, &mut conversation, start);
        headset.session(SessionState::Visible, &mut conversation, start);
        assert!(!headset.microphone_due(true, start + SETTLE * 4));
        // Focused, as the launch settles, but nothing shown yet: still not asked.
        headset.session(SessionState::Focused, &mut conversation, start);
        assert!(!headset.microphone_due(false, start + SETTLE * 5));
        // Shown with focus for a moment: asked, and the panel says so.
        let shown = start + SETTLE * 6;
        assert!(!headset.microphone_due(true, shown));
        assert!(headset.microphone_due(true, shown + SETTLE));
        let asked = shown + SETTLE;
        headset.microphone_answered(Ok(false), &mut conversation, asked);
        assert_eq!(
            conversation.menu_model().notice.as_deref(),
            Some(MICROPHONE_REQUEST)
        );
        // The dialog takes focus, and the session stops and begins again while it is up; frames go on and only check.
        headset.session(SessionState::Visible, &mut conversation, asked);
        assert!(headset.microphone_due(true, asked + CHECK));
        headset.microphone_answered(Ok(false), &mut conversation, asked + CHECK);
        headset.session(SessionState::Stopping, &mut conversation, asked + CHECK);
        assert!(headset.microphone_due(false, asked + CHECK * 2));
        headset.microphone_answered(Ok(false), &mut conversation, asked + CHECK * 2);
        headset.session(SessionState::Ready, &mut conversation, asked + CHECK * 2);
        headset.session(SessionState::Focused, &mut conversation, asked + CHECK * 2);
        // Allowed: the next check sees it, and the panel no longer waits.
        assert!(headset.microphone_due(true, asked + CHECK * 3));
        headset.microphone_answered(Ok(true), &mut conversation, asked + CHECK * 3);
        assert_eq!(conversation.menu_model().notice, None);
        assert!(!headset.microphone_due(true, asked + CHECK * 10));

        // Denied: once focus has been back a moment, the panel no longer waits, and the first recording asks again.
        let mut headset = Headset::new(&mut conversation, &mut feedback, start);
        headset.session(SessionState::Ready, &mut conversation, start);
        headset.session(SessionState::Focused, &mut conversation, start);
        headset.microphone_due(true, start);
        assert!(headset.microphone_due(true, start + SETTLE));
        headset.microphone_answered(Ok(false), &mut conversation, start + SETTLE);
        headset.session(SessionState::Visible, &mut conversation, start + SETTLE);
        let returned = start + SETTLE + CHECK;
        headset.session(SessionState::Focused, &mut conversation, returned);
        assert!(headset.microphone_due(true, returned));
        headset.microphone_answered(Ok(false), &mut conversation, returned);
        assert!(conversation.menu_model().notice.is_some());
        assert!(headset.microphone_due(true, returned + RETURNED));
        headset.microphone_answered(Ok(false), &mut conversation, returned + RETURNED);
        assert_eq!(conversation.menu_model().notice, None);

        // A platform that cannot be asked is reported once, and the panel does not wait.
        let mut headset = Headset::new(&mut conversation, &mut feedback, start);
        headset.session(SessionState::Ready, &mut conversation, start);
        headset.session(SessionState::Focused, &mut conversation, start);
        headset.microphone_due(true, start);
        assert!(headset.microphone_due(true, start + SETTLE));
        headset.microphone_answered(Err("no activity".into()), &mut conversation, start);
        assert_eq!(conversation.menu_model().notice, None);
        assert!(!headset.microphone_due(true, start + SETTLE * 2));
    }

    #[test]
    fn the_microphone_request_notice_gives_way_to_another() {
        let mut conversation = joined();
        conversation.security_notice("The server's identity changed.");
        conversation.microphone_request(true);
        conversation.microphone_request(false);
        assert_eq!(
            conversation.menu_model().notice.as_deref(),
            Some("The server's identity changed.")
        );
    }

    #[test]
    fn idle_frames_let_go_of_what_is_held_until_focused_and_running() {
        let mut conversation = joined();
        let mut feedback = felt();
        let start = Instant::now();
        let mut headset = Headset::new(&mut conversation, &mut feedback, start);
        headset.session(SessionState::Ready, &mut conversation, start);
        headset.session(SessionState::Focused, &mut conversation, start);
        let talk = Controls {
            talk: true,
            ..Controls::default()
        };
        headset.idle(&mut conversation);
        headset.buttons(&talk, &mut conversation, &mut feedback, start);
        assert!(conversation.recording().is_some(), "focused and running");
        headset.session(SessionState::Stopping, &mut conversation, start);
        headset.idle(&mut conversation);
        assert!(conversation.recording().is_none());
    }

    #[test]
    fn a_frame_measures_the_time_since_the_last() {
        let mut conversation = joined();
        let start = Instant::now();
        let mut headset = Headset::new(&mut conversation, &mut felt(), start);
        assert_eq!(headset.tick(start + Duration::from_millis(250)), 0.25);
        headset.session(
            SessionState::Ready,
            &mut conversation,
            start + Duration::from_secs(1),
        );
        assert_eq!(headset.tick(start + Duration::from_millis(1500)), 0.5);
    }

    #[test]
    fn a_firm_left_squeeze_toggles_the_live_microphone_once() {
        let mut conversation = joined();
        let mut feedback = felt();
        let start = Instant::now();
        let mut headset = Headset::new(&mut conversation, &mut feedback, start);
        headset.session(SessionState::Focused, &mut conversation, start);
        let grip = |value| Controls {
            audience_grip: value,
            audience_grip_active: true,
            ..Controls::default()
        };
        headset.buttons(&grip(0.7), &mut conversation, &mut feedback, start);
        assert!(!conversation.live(), "not firm enough");
        headset.buttons(&grip(0.9), &mut conversation, &mut feedback, start);
        assert!(conversation.live());
        headset.buttons(&grip(0.9), &mut conversation, &mut feedback, start);
        assert!(conversation.live(), "held, it toggles once");
        headset.buttons(&grip(0.0), &mut conversation, &mut feedback, start);
        headset.buttons(&grip(0.9), &mut conversation, &mut feedback, start);
        assert!(!conversation.live());
    }

    #[test]
    fn vibrations_are_given_only_while_focused_and_never_late() {
        let mut conversation = joined();
        let mut feedback = Feedback::new(Settings::default());
        let start = Instant::now();
        let mut headset = Headset::new(&mut conversation, &mut feedback, start);
        feedback.emit(tomori_client::feedback::Event::Choose, start);
        assert!(
            headset.haptics(&mut feedback, start).is_empty(),
            "unfocused"
        );
        assert!(
            headset.haptics(&mut feedback, start).is_empty(),
            "and dropped"
        );
        headset.session(SessionState::Focused, &mut conversation, start);
        feedback.emit(tomori_client::feedback::Event::Choose, start);
        assert_eq!(headset.haptics(&mut feedback, start).len(), 1);
    }

    /// The right controller's grip, low in front and to the right, turned by `turn` from its neutral hold.
    fn grip(turn: Quat) -> Pose {
        Pose {
            position: [0.2, 1.1, -0.35],
            orientation: turn.to_array(),
        }
    }

    /// Rolling the right hand outwards about the forward axis by 90° turns the palm up; `degrees` more turns it back.
    fn palm_up_by(degrees: f32) -> Quat {
        Quat::from_rotation_z(degrees.to_radians()) * Quat::from_rotation_z(-90f32.to_radians())
    }

    struct Rig {
        headset: Headset,
        conversation: Conversation,
        feedback: Feedback,
        views: [View; 2],
        start: Instant,
    }

    impl Rig {
        fn new(conversation: Conversation) -> Self {
            let mut conversation = conversation;
            let mut feedback = felt();
            let start = Instant::now();
            let mut headset = Headset::new(&mut conversation, &mut feedback, start);
            headset.session(SessionState::Ready, &mut conversation, start);
            headset.session(SessionState::Focused, &mut conversation, start);
            Self {
                headset,
                conversation,
                feedback,
                views: stereo(Vec3::new(0.0, 1.6, 0.0), Quat::IDENTITY),
                start,
            }
        }

        fn frame(
            &mut self,
            controls: &Controls,
            hands: [Option<[Joint; hands::JOINT_COUNT]>; 2],
            at: Duration,
        ) -> Drawn<'_> {
            let now = self.start + at;
            self.headset
                .buttons(controls, &mut self.conversation, &mut self.feedback, now);
            self.headset.update(
                &self.views,
                at.as_nanos() as i64,
                controls,
                hands,
                &mut self.conversation,
                &mut self.feedback,
                now,
                FRAME,
            )
        }
    }

    fn right_grip(turn: Quat) -> Controls {
        Controls {
            controller: Some(Vec3::from_array(grip(turn).position)),
            controller_poses: [None, Some(grip(turn))],
            ..Controls::default()
        }
    }

    #[test]
    fn the_right_controller_rolled_palm_up_shows_the_transcript_after_the_dwell_and_hides_past_55_degrees()
     {
        let mut rig = Rig::new(joined());
        for (tilt, shown, at) in [
            (40.0, false, 0),
            (40.0, false, 100),
            (40.0, true, 200),
            (-50.0, true, 300),
            (50.0, true, 400),
            (60.0, false, 500),
            (50.0, false, 600),
            (50.0, false, 900),
            (0.0, false, 1000),
            (0.0, true, 1200),
        ] {
            let drawn = rig.frame(
                &right_grip(palm_up_by(tilt)),
                [None, None],
                Duration::from_millis(at),
            );
            assert_eq!(
                drawn.frame.panels.wrist.model.is_some(),
                shown,
                "{tilt}° at {at} ms"
            );
            // The controller's model is drawn, so the attractor is not.
            assert_eq!(drawn.frame.marker, None);
            assert!(drawn.controllers[controller::RIGHT].is_some());
        }
        // Opening it on the wrist is felt on the right hand.
        let felt = rig
            .headset
            .haptics(&mut rig.feedback, rig.start + Duration::from_secs(2));
        assert!(felt.iter().any(|(side, _)| *side == Side::Right));
        // Pulling the trigger keeps it from showing.
        let mut rig = Rig::new(joined());
        let pulling = Controls {
            trigger: Some(0.5),
            ..right_grip(palm_up_by(0.0))
        };
        for at in [0, 200, 400] {
            let drawn = rig.frame(&pulling, [None, None], Duration::from_millis(at));
            assert!(drawn.frame.panels.wrist.model.is_none());
        }
    }

    #[test]
    fn the_left_controller_rolled_palm_up_shows_the_menu_as_the_right_shows_the_transcript() {
        let mut rig = Rig::new(joined());
        // The left hand rolls outwards the other way, so its grip mirrors the right's, low in front and to the left.
        let left_grip = |degrees: f32| {
            let turn = Quat::from_rotation_z(-degrees.to_radians())
                * Quat::from_rotation_z(90f32.to_radians());
            Controls {
                controller_poses: [
                    Some(Pose {
                        position: [-0.2, 1.1, -0.35],
                        orientation: turn.to_array(),
                    }),
                    None,
                ],
                ..Controls::default()
            }
        };
        let neutral = Controls {
            controller_poses: [
                Some(Pose {
                    position: [-0.2, 1.1, -0.35],
                    orientation: Quat::IDENTITY.to_array(),
                }),
                None,
            ],
            ..Controls::default()
        };
        for (controls, shown, at) in [
            (&neutral, false, 0),
            (&neutral, false, 300),
            (&left_grip(40.0), false, 400),
            (&left_grip(40.0), false, 500),
            (&left_grip(40.0), true, 600),
            (&left_grip(-50.0), true, 700),
            (&left_grip(60.0), false, 800),
            (&left_grip(50.0), false, 900),
            (&left_grip(50.0), false, 1200),
        ] {
            let drawn = rig.frame(controls, [None, None], Duration::from_millis(at));
            assert_eq!(
                drawn
                    .frame
                    .arrival
                    .is_some_and(|arrival| !arrival.scene.objects.is_empty()),
                shown,
                "at {at} ms"
            );
        }
    }

    #[test]
    fn the_transcript_faces_the_middle_of_the_head() {
        let mut rig = Rig::new(joined());
        rig.frame(&right_grip(palm_up_by(0.0)), [None, None], Duration::ZERO);
        let drawn = rig.frame(&right_grip(palm_up_by(0.0)), [None, None], HOLD);
        let model = drawn.frame.panels.wrist.model.expect("shown");
        let centre = model.transform_point3(Vec3::ZERO);
        let normal = model.transform_vector3(Vec3::Z).normalize();
        let head = hand_menu::head_centre(Mat4::from_translation(Vec3::new(0.0, 1.6, 0.0)));
        assert!(normal.dot((head - centre).normalize()) > 0.999);
    }

    #[test]
    fn without_a_right_controller_the_attractor_is_drawn_where_the_trigger_pulls() {
        let mut rig = Rig::new(joined());
        let position = Vec3::new(0.2, 1.0, -0.4);
        let controls = Controls {
            controller: Some(position),
            trigger: Some(1.0),
            ..Controls::default()
        };
        let drawn = rig.frame(&controls, [None, None], Duration::ZERO);
        assert!(drawn.frame.attracting);
        assert_eq!(drawn.frame.marker, Some(position));
        let (scene_to_space, projection) = (drawn.scene_to_space, drawn.projections[0]);
        assert_eq!(scene_to_space, rig.headset.anchor.transform());
        assert_eq!(projection, view_projection(&rig.views[0]));
    }

    #[test]
    fn y_pins_the_menu_above_the_left_controller_facing_the_head() {
        let mut rig = Rig::new(joined());
        let left = Pose {
            position: [-0.2, 1.1, -0.35],
            orientation: Quat::from_rotation_z(1.2).to_array(),
        };
        let mut controls = Controls {
            refresh: true,
            controller_poses: [Some(left), None],
            ..Controls::default()
        };
        let drawn = rig.frame(&controls, [None, None], Duration::ZERO);
        let arrival = drawn.frame.arrival.expect("the pinned menu shows at once");
        assert!(!arrival.scene.objects.is_empty());
        // Its rows stand above the controller, whatever its angle.
        assert!(
            arrival
                .scene
                .objects
                .iter()
                .all(|object| object.position.y > left.position[1])
        );
        controls.refresh = false;
        let presence = rig
            .frame(&controls, [None, None], Duration::from_millis(10))
            .frame
            .presence
            .expect("a participant");
        assert!(presence.left.is_some() && presence.right.is_none());
    }

    #[test]
    fn a_pinned_menu_needs_somewhere_to_face_from() {
        let mut rig = Rig::new(joined());
        // The left controller holds the menu's spot exactly at the middle of the head, so it has no way to face.
        let head = hand_menu::head_centre(head_pose(&rig.views));
        let left = Pose {
            position: (head - Vec3::new(0.0, 0.03, 0.0)).to_array(),
            orientation: [0.0, 0.0, 0.0, 1.0],
        };
        let controls = Controls {
            refresh: true,
            controller_poses: [Some(left), None],
            ..Controls::default()
        };
        let drawn = rig.frame(&controls, [None, None], Duration::ZERO);
        assert!(
            drawn
                .frame
                .arrival
                .is_none_or(|arrival| arrival.scene.objects.is_empty())
        );
        assert!(rig.headset.step.pinned());
    }

    #[test]
    fn a_tracked_left_hand_turned_palm_up_shows_the_menu() {
        let mut rig = Rig::new(joined());
        // A left hand held up in front, its palm up: the little finger crossed with the index points out of the palm.
        let wrist = Vec3::new(0.0, 1.55, -0.35);
        let mut joints = [Joint::default(); hands::JOINT_COUNT];
        for (index, joint) in joints.iter_mut().enumerate() {
            *joint = Joint {
                position: wrist + Vec3::new(0.0, 0.01 * index as f32, 0.0),
                radius: 0.01,
                position_valid: true,
            };
        }
        joints[1].position = wrist;
        joints[7].position = wrist + Vec3::new(0.0, 0.0, -0.08);
        joints[22].position = wrist + Vec3::new(0.07, 0.0, 0.0);
        let hands = [Some(joints), None];
        rig.frame(&Controls::default(), hands, Duration::ZERO);
        let drawn = rig.frame(&Controls::default(), hands, HOLD);
        assert!(drawn.hands[controller::LEFT].is_some());
        assert!(
            drawn
                .frame
                .arrival
                .is_some_and(|arrival| !arrival.scene.objects.is_empty())
        );
        // A tracked right hand turned palm up shows the transcript, and hides the attractor.
        let mut rig = Rig::new(joined());
        let mut right = joints;
        right[7].position = wrist + Vec3::new(0.0, 0.0, -0.08);
        right[22].position = wrist + Vec3::new(-0.07, 0.0, 0.0);
        let hands = [None, Some(right)];
        let drawn = rig.frame(&Controls::default(), hands, Duration::ZERO);
        assert_eq!(drawn.frame.marker, None);
        assert!(drawn.frame.panels.wrist.model.is_none());
        let drawn = rig.frame(&Controls::default(), hands, HOLD);
        assert!(drawn.frame.panels.wrist.model.is_some());
    }

    #[test]
    fn the_aim_points_the_ray_and_a_pull_on_nothing_attracts() {
        let mut rig = Rig::new(lobby(ConnectionRole::Owner));
        let controls = Controls {
            aim: Some(Pose {
                position: [0.2, 1.2, -0.3],
                orientation: [0.0, 0.0, 0.0, 1.0],
            }),
            trigger: Some(1.0),
            ..Controls::default()
        };
        let drawn = rig.frame(&controls, [None, None], Duration::ZERO);
        assert!(drawn.frame.attracting);
        // In arrival the ray is drawn from the controller, straight ahead.
        let beam = drawn
            .frame
            .arrival
            .and_then(|arrival| arrival.ray)
            .expect("the beam");
        let scene_from_space = rig.headset.anchor.transform().inverse();
        let origin = scene_from_space.transform_point3(Vec3::new(0.2, 1.2, -0.3));
        assert!(beam.origin.distance(origin) < 1e-4, "{}", beam.origin);
        let ahead = scene_from_space.transform_vector3(-Vec3::Z);
        assert!((beam.end - beam.origin).normalize().dot(ahead) > 0.999);
        // Unfocused, the trigger does nothing.
        rig.headset
            .session(SessionState::Visible, &mut rig.conversation, rig.start);
        let unfocused = Controls::from_actions(actions(true, true, true), false);
        let drawn = rig.frame(&unfocused, [None, None], Duration::from_millis(10));
        assert!(!drawn.frame.attracting);
    }

    #[test]
    fn a_recentre_waits_for_its_display_time() {
        let mut rig = Rig::new(joined());
        rig.views = stereo(Vec3::new(1.0, 1.6, 0.0), Quat::from_rotation_y(0.5));
        rig.frame(&Controls::default(), [None, None], Duration::from_nanos(10));
        let aligned = rig.headset.anchor.transform();
        assert_ne!(aligned, Mat4::IDENTITY);
        rig.headset.recentre(30);
        assert_eq!(
            rig.frame(&Controls::default(), [None, None], Duration::from_nanos(20))
                .scene_to_space,
            aligned
        );
        assert_eq!(
            rig.frame(&Controls::default(), [None, None], Duration::from_nanos(30))
                .scene_to_space,
            Mat4::IDENTITY
        );
    }

    /// A space in Sky, declared for this test to teleport onto a floor 20 m across.
    fn travelling() -> Conversation {
        let mut conversation = joined();
        conversation.set_locomotion(vec![(
            crate::environment::Environment::SKY,
            crate::locomotion::Locomotion::teleport(vec![crate::locomotion::Surface::rectangle(
                Vec3::ZERO,
                glam::Vec2::splat(20.0),
            )]),
        )]);
        conversation
    }

    #[test]
    fn the_left_stick_teleports_and_turns_and_every_pose_follows() {
        let mut rig = Rig::new(travelling());
        // Travel waits for the space's scenery to grow in, so that its surfaces are seen.
        for _ in 0..400 {
            rig.frame(&Controls::default(), [None, None], Duration::ZERO);
        }
        // The right controller points down and ahead, at the floor 1.5 m in front.
        let aim = Pose {
            position: [0.2, 1.2, -0.3],
            orientation: Quat::from_rotation_x(-45f32.to_radians()).to_array(),
        };
        let right = grip(Quat::IDENTITY);
        let pointing = |stick: glam::Vec2| Controls {
            aim: Some(aim),
            controller_poses: [None, Some(right)],
            navigation: stick,
            navigation_active: true,
            trigger: Some(0.0),
            ..Controls::default()
        };
        let drawn = rig.frame(&pointing(glam::Vec2::Y), [None, None], Duration::ZERO);
        let landing = drawn.frame.landing.expect("somewhere to land");
        assert!(landing.distance(Vec3::new(0.2, 0.0, -1.5)) < 1e-4);
        let beam = drawn.frame.arrival.and_then(|arrival| arrival.ray).unwrap();
        assert!(beam.hit && beam.end.distance(landing) < 1e-4);
        // Finding a landing ticks, on the right.
        let now = rig.start;
        let ticks = rig.headset.haptics(&mut rig.feedback, now);
        assert!(matches!(ticks.as_slice(), [(Side::Right, _)]), "{ticks:?}");
        // Releasing the stick lands there, with a click, and this frame is still drawn from where the participant was.
        let drawn = rig.frame(&pointing(glam::Vec2::ZERO), [None, None], HOLD);
        assert_eq!(drawn.frame.landing, None);
        let head = drawn.frame.presence.unwrap().head;
        assert!(Vec3::from_array(head.position).distance(Vec3::new(0.0, 1.6, 0.0)) < 1e-4);
        // That frame is drawn dark, from where the poses were placed, so that the view does not jump.
        assert_eq!(drawn.frame.blink, 1.0);
        assert_eq!(drawn.frame.rig, crate::locomotion::Rig::default());
        assert_eq!(
            drawn.frame.presence.unwrap().rig,
            crate::locomotion::Rig::default()
        );
        assert_eq!(
            rig.headset
                .haptics(&mut rig.feedback, rig.start + HOLD)
                .len(),
            1
        );
        // From the next frame, everything is where the participant travelled, a tracked hand too.
        let wrist = Vec3::new(-0.2, 1.1, -0.3);
        let mut joints = [Joint::default(); hands::JOINT_COUNT];
        for (index, joint) in joints.iter_mut().enumerate() {
            *joint = Joint {
                position: wrist + Vec3::new(0.0, 0.01 * index as f32, 0.0),
                radius: 0.01,
                position_valid: true,
            };
        }
        joints[1].position = wrist;
        let drawn = rig.frame(&pointing(glam::Vec2::ZERO), [Some(joints), None], HOLD * 2);
        let travelled = Vec3::new(0.2, 1.6, -1.5);
        // It comes back from dark, and the frame says where the room now is, for the voices around it.
        assert!(drawn.frame.blink > 0.0 && drawn.frame.blink < 1.0);
        assert!(
            drawn
                .frame
                .rig
                .matrix()
                .transform_point3(Vec3::new(0.0, 1.6, 0.0))
                .distance(travelled)
                < 1e-4
        );
        let presence = drawn.frame.presence.unwrap();
        // Presence carries the rig its poses were placed with, so that the others can tell this travel from walking.
        assert_eq!(presence.rig, drawn.frame.rig);
        assert_ne!(presence.rig, crate::locomotion::Rig::default());
        let hand = presence.hands[controller::LEFT].unwrap().joints[1].unwrap();
        assert!(Vec3::from_array(hand.position).distance(wrist + Vec3::new(0.2, 0.0, -1.5)) < 1e-4);
        assert!(Vec3::from_array(presence.head.position).distance(travelled) < 1e-4);
        assert_eq!(drawn.frame.listener, Some(presence.head));
        let controller = Vec3::from_array(presence.right.unwrap().position);
        assert!(
            controller.distance(Vec3::from_array(right.position) + Vec3::new(0.2, 0.0, -1.5))
                < 1e-4
        );
        assert_eq!(drawn.controllers[controller::RIGHT], presence.right);
        // The eyes see the scene from there: the scene's travelled head is the reference space's head.
        let seen = drawn.scene_to_space.transform_point3(travelled);
        assert!(seen.distance(Vec3::new(0.0, 1.6, 0.0)) < 1e-4);
        // Right turns 30° about the head, which stays where it is.
        rig.frame(&pointing(glam::Vec2::X), [None, None], HOLD * 3);
        let drawn = rig.frame(&pointing(glam::Vec2::ZERO), [None, None], HOLD * 4);
        let head = drawn.frame.presence.unwrap().head;
        assert!(Vec3::from_array(head.position).distance(travelled) < 1e-4);
        let forward = Quat::from_array(head.orientation) * -Vec3::Z;
        assert!(forward.distance(Vec3::new(0.5, 0.0, -(3f32.sqrt()) / 2.0)) < 1e-4);
        // A left controller lost while aiming, whose stick then reads zero, lands nowhere.
        rig.frame(&pointing(glam::Vec2::Y), [None, None], HOLD * 5);
        let lost = Controls {
            navigation_active: false,
            ..pointing(glam::Vec2::ZERO)
        };
        rig.frame(&lost, [None, None], HOLD * 6);
        let drawn = rig.frame(&pointing(glam::Vec2::ZERO), [None, None], HOLD * 7);
        let head = drawn.frame.presence.unwrap().head;
        assert!(Vec3::from_array(head.position).distance(travelled) < 1e-4);
        // A recentre keeps where the participant travelled.
        rig.headset.recentre(0);
        let drawn = rig.frame(&pointing(glam::Vec2::ZERO), [None, None], HOLD * 5);
        let head = drawn.frame.presence.unwrap().head;
        assert!(Vec3::from_array(head.position).distance(travelled) < 1e-4);
    }

    #[test]
    fn without_a_declared_locomotion_the_left_stick_changes_nothing() {
        let mut rig = Rig::new(joined());
        let controls = |stick: glam::Vec2| Controls {
            aim: Some(Pose {
                position: [0.2, 1.2, -0.3],
                orientation: Quat::from_rotation_x(-45f32.to_radians()).to_array(),
            }),
            navigation: stick,
            ..Controls::default()
        };
        for (at, stick) in [
            glam::Vec2::Y,
            glam::Vec2::ZERO,
            glam::Vec2::X,
            glam::Vec2::ZERO,
        ]
        .into_iter()
        .enumerate()
        {
            let drawn = rig.frame(&controls(stick), [None, None], HOLD * at as u32);
            assert_eq!(drawn.frame.landing, None);
            let head = drawn.frame.presence.unwrap().head;
            assert_eq!(head.position, [0.0, 1.6, 0.0]);
            assert_eq!(head.orientation, [0.0, 0.0, 0.0, 1.0]);
        }
    }
}
