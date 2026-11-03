// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The desktop window as a participant: it connects to a Tomori server and presents arrival, the space and the hand menu as the headset does, seen from a head at standing height that the mouse turns, with the keyboard for the controller's buttons and the mouse for its ray. See `design/desktop.md`.

use crate::{
    arrival, arrival_graphics, arrival_motion,
    caption::Caption,
    connection::Connection,
    controller,
    conversation::Conversation,
    desktop::{WindowGpu, acquire},
    graphics::Renderer,
    hand_menu::{HandMenu, Palm},
    pointing::{Pointer, Ray},
    swarm::Swarm,
};
use anyhow::Result;
use glam::{EulerRot, Mat4, Quat, Vec2, Vec3};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use winit::{
    application::ApplicationHandler,
    dpi::{LogicalSize, PhysicalSize},
    event::{ElementState, KeyEvent, MouseButton, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::{Window, WindowId},
};

const TITLE: &str = "Tomori";

/// What each key does, shown at startup and by `--help`; the keys stand for the headset's controls in `design/interaction.md`.
pub const CONTROLS: &str = "Mouse: point (as the right controller's ray); left button: choose what is pointed at, or attract the swarm; right drag: look around.\n\
Enter: A. Backspace: B (hold for a second to leave the space). Space (hold): X, push-to-talk. Tab: Y.\n\
Arrow keys: left stick. [ and ]: right stick left and right. Page Up and Page Down: right stick up and down.\n\
V: mute spoken replies (left stick click). R: play the latest reply again, or stop it (right stick click).\n\
M: open or close the hand menu. K: knock on the space you watch. + and -: admit or decline the first knock. P: pair this device as an owner, or cancel.\n\
Esc: close the hand menu, or quit.";

/// How far the stick must move before it counts, as on the headset.
const STICK: f32 = 0.6;
/// How far the head turns, in radians, for each pixel the mouse is dragged.
const LOOK_SPEED: f32 = 0.004;

/// Open a desktop window connected as `conversation` and `connection` describe, until it is closed.
pub fn run(conversation: Conversation, connection: Connection) -> Result<()> {
    let event_loop = EventLoop::new()?;
    let mut app = App {
        participant: Some(Participant::new(conversation, connection)),
        state: None,
        error: None,
    };
    log::info!("{CONTROLS}");
    event_loop.run_app(&mut app)?;
    match app.error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

struct App {
    /// Taken into `state` once the window exists.
    participant: Option<Participant>,
    state: Option<State>,
    error: Option<anyhow::Error>,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        let result = (|| {
            let attributes = Window::default_attributes()
                .with_title(TITLE)
                .with_inner_size(LogicalSize::new(1280.0, 800.0));
            let window = Arc::new(event_loop.create_window(attributes)?);
            let participant = self.participant.take().expect("the participant, once");
            pollster::block_on(State::new(window, participant))
        })();
        match result {
            Ok(state) => {
                state.window.request_redraw();
                self.state = Some(state);
            }
            Err(error) => {
                self.error = Some(error);
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        let Some(state) = &mut self.state else {
            return;
        };
        if id != state.window.id() {
            return;
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => state.resize(size),
            WindowEvent::Focused(focused) => state.participant.set_focused(focused),
            WindowEvent::Occluded(occluded) => {
                state.occluded = occluded;
                state.participant.set_visible(!occluded);
                if !occluded {
                    state.window.request_redraw();
                }
            }
            WindowEvent::CursorLeft { .. } => {
                state.participant.input.cursor = None;
                state.participant.input.looking = false;
            }
            WindowEvent::CursorMoved { position, .. } => {
                let cursor = Vec2::new(position.x as f32, position.y as f32);
                let input = &mut state.participant.input;
                if input.looking
                    && let Some(last) = input.cursor
                {
                    state.participant.head.look(cursor - last);
                }
                state.participant.input.cursor = Some(cursor);
            }
            WindowEvent::MouseInput {
                state: pressed,
                button,
                ..
            } => {
                let pressed = pressed == ElementState::Pressed;
                match button {
                    MouseButton::Left => state.participant.input.trigger = pressed,
                    MouseButton::Right => state.participant.input.looking = pressed,
                    _ => {}
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if state.participant.key(&event) == KeyResult::Quit {
                    event_loop.exit();
                }
            }
            WindowEvent::RedrawRequested => {
                if state.occluded || state.size.width == 0 || state.size.height == 0 {
                    return;
                }
                if let Err(error) = state.frame(true) {
                    self.error = Some(error);
                    event_loop.exit();
                    return;
                }
                state.window.request_redraw();
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // A hidden window draws nothing, so it would never receive a redraw; it still keeps its connection and speech going.
        let Some(state) = &mut self.state else {
            return;
        };
        if state.occluded || state.size.width == 0 || state.size.height == 0 {
            if let Err(error) = state.frame(false) {
                self.error = Some(error);
                event_loop.exit();
                return;
            }
            event_loop.set_control_flow(ControlFlow::WaitUntil(
                Instant::now() + Duration::from_millis(20),
            ));
        } else {
            event_loop.set_control_flow(ControlFlow::Wait);
        }
    }
}

#[derive(PartialEq)]
enum KeyResult {
    Handled,
    Quit,
}

/// The keyboard and mouse as the headset's controls; see `CONTROLS`.
#[derive(Default)]
struct Input {
    cursor: Option<Vec2>,
    trigger: bool,
    looking: bool,
    talk: bool,
    back_held: bool,
    /// Presses since the last frame, which it consumes.
    a: bool,
    back: bool,
    y: bool,
    mute: bool,
    replay: bool,
    left: [bool; 4],
    right: [bool; 4],
}

impl Input {
    /// Release everything held, as on losing focus.
    fn release(&mut self) {
        let cursor = self.cursor;
        *self = Self {
            cursor,
            ..Self::default()
        };
    }

    fn stick(keys: [bool; 4]) -> Vec2 {
        let [up, down, left, right] = keys.map(f32::from);
        Vec2::new(right - left, up - down)
    }
}

/// The participant's head, standing at arrival's home and turned by the mouse.
struct Head {
    yaw: f32,
    pitch: f32,
}

impl Default for Head {
    fn default() -> Self {
        // Arrival is laid out ahead, along -Z, and a little below the eyes.
        Self {
            yaw: 0.0,
            pitch: -0.12,
        }
    }
}

impl Head {
    fn look(&mut self, delta: Vec2) {
        self.yaw -= delta.x * LOOK_SPEED;
        self.pitch = (self.pitch - delta.y * LOOK_SPEED).clamp(-1.4, 1.4);
    }

    fn rotation(&self) -> Quat {
        Quat::from_euler(EulerRot::YXZ, self.yaw, self.pitch, 0.0)
    }

    /// The head's pose in the scene, as the headset's views give it.
    fn transform(&self) -> Mat4 {
        Mat4::from_rotation_translation(self.rotation(), arrival::HOME)
    }

    fn view_projection(&self, aspect: f32) -> Mat4 {
        Mat4::perspective_rh(60_f32.to_radians(), aspect, 0.05, 50.0) * self.transform().inverse()
    }

    /// The ray from the eye through the cursor, as the right controller's aim on the headset.
    fn ray(&self, cursor: Vec2, size: Vec2) -> Option<Ray> {
        let ndc = Vec2::new(cursor.x / size.x * 2.0 - 1.0, 1.0 - cursor.y / size.y * 2.0);
        let inverse = self.view_projection(size.x / size.y).inverse();
        let near = inverse.project_point3(ndc.extend(0.0));
        let far = inverse.project_point3(ndc.extend(1.0));
        Ray::new(near, far - near)
    }

    /// A left palm held up in front of the participant, facing them, where the hand menu shows while it is open.
    fn palm(&self) -> Palm {
        let wrist = arrival::HOME + self.rotation() * Vec3::new(-0.06, -0.14, -0.45);
        Palm {
            wrist,
            normal: (arrival::HOME - wrist).normalize(),
        }
    }
}

/// The conversation with the server, and everything the headset's frame loop keeps between frames, without the headset.
struct Participant {
    conversation: Conversation,
    connection: Connection,
    input: Input,
    head: Head,
    focused: bool,
    visible: bool,
    engagement: client::engagement::Tracker,
    microphone_requested: bool,
    menu_open: bool,
    stick_latched: bool,
    choice_latched: bool,
    queue_latched: bool,
    back_since: Option<Instant>,
    was_talking: bool,
    talk_blocked: bool,
    highlighted: Option<arrival::Key>,
    menu_was_shown: bool,
    knocks: usize,
}

impl Participant {
    fn new(mut conversation: Conversation, connection: Connection) -> Self {
        let engagement = client::engagement::Tracker::new();
        conversation.set_engagement(engagement.engagement());
        Self {
            conversation,
            connection,
            input: Input::default(),
            head: Head::default(),
            focused: false,
            visible: true,
            engagement,
            microphone_requested: false,
            menu_open: false,
            stick_latched: false,
            choice_latched: false,
            queue_latched: false,
            back_since: None,
            was_talking: false,
            talk_blocked: false,
            highlighted: None,
            menu_was_shown: false,
            knocks: 0,
        }
    }

    fn set_focused(&mut self, focused: bool) {
        if self.focused && !focused {
            // As the headset does on losing focus: nothing held carries over.
            self.input.release();
            self.stick_latched = false;
            self.choice_latched = false;
            self.queue_latched = false;
            self.back_since = None;
            self.conversation.hold_back(None);
            self.was_talking = false;
            self.talk_blocked = true;
            self.conversation.stop_live();
            self.conversation.stop_talking();
            self.highlighted = None;
        }
        self.focused = focused;
        if focused && !self.microphone_requested {
            self.microphone_requested = true;
            self.connection.request_microphone();
        }
        self.report_engagement();
    }

    fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
        self.report_engagement();
    }

    /// A focused window has input, an unfocused one is only seen, and a hidden one is neither; a desktop has no sensor for being worn.
    fn report_engagement(&mut self) {
        use client::engagement::Focus;
        let focus = match (self.visible, self.focused) {
            (false, _) => Focus::Hidden,
            (true, true) => Focus::Focused,
            (true, false) => Focus::Visible,
        };
        if let Some(changed) = self.engagement.set_focus(focus) {
            log::info!("Engagement: {changed:?}, as the window is {focus:?}");
            self.conversation.set_engagement(changed);
        }
    }

    fn key(&mut self, event: &KeyEvent) -> KeyResult {
        let PhysicalKey::Code(code) = event.physical_key else {
            return KeyResult::Handled;
        };
        let pressed = event.state == ElementState::Pressed;
        let press = pressed && !event.repeat;
        let input = &mut self.input;
        match code {
            KeyCode::Space => input.talk = pressed,
            KeyCode::Backspace => {
                input.back_held = pressed;
                input.back |= press;
            }
            KeyCode::Enter | KeyCode::NumpadEnter => input.a |= press,
            KeyCode::Tab => input.y |= press,
            KeyCode::KeyV => input.mute |= press,
            KeyCode::KeyR => input.replay |= press,
            KeyCode::ArrowUp => input.left[0] = pressed,
            KeyCode::ArrowDown => input.left[1] = pressed,
            KeyCode::ArrowLeft => input.left[2] = pressed,
            KeyCode::ArrowRight => input.left[3] = pressed,
            KeyCode::PageUp => input.right[0] = pressed,
            KeyCode::PageDown => input.right[1] = pressed,
            KeyCode::BracketLeft => input.right[2] = pressed,
            KeyCode::BracketRight => input.right[3] = pressed,
            KeyCode::KeyM if press => self.menu_open = !self.menu_open,
            KeyCode::KeyK if press => self.conversation.knock(),
            KeyCode::KeyP if press => self.conversation.pair(),
            KeyCode::Equal | KeyCode::NumpadAdd | KeyCode::Minus | KeyCode::NumpadSubtract
                if press =>
            {
                let admit = matches!(code, KeyCode::Equal | KeyCode::NumpadAdd);
                match self
                    .conversation
                    .knocks()
                    .first()
                    .map(|knock| knock.id.clone())
                {
                    Some(id) => self.conversation.decide_knock(&id, admit),
                    None => log::info!("No knock is waiting."),
                }
            }
            KeyCode::Escape if press => {
                if !self.menu_open {
                    return KeyResult::Quit;
                }
                self.menu_open = false;
            }
            _ => {}
        }
        KeyResult::Handled
    }
}

struct State {
    window: Arc<Window>,
    gpu: WindowGpu,
    renderer: Renderer,
    caption: Caption,
    size: PhysicalSize<u32>,
    occluded: bool,
    participant: Participant,
    swarm: Swarm,
    signals: crate::interaction::Signals,
    pointer: Pointer<arrival::Key>,
    hand_menu: HandMenu,
    motion: arrival_motion::Motion,
    arrival_time: f32,
    delegation_scene: crate::delegation::Scene,
    workers: crate::delegation::Workers,
    coordinator_samples: Vec<[f32; 3]>,
    presentation_navigation: crate::workspace_graphics::CardNavigation,
    panel: crate::panel_anchor::PanelAnchor,
    last_frame: Instant,
    first_frame: bool,
}

impl State {
    async fn new(window: Arc<Window>, participant: Participant) -> Result<Self> {
        let gpu = WindowGpu::new(window.clone()).await?;
        let size = window.inner_size();
        let samples = gpu.samples(gpu.config.format);
        let renderer = Renderer::with_samples(
            &gpu.device,
            &gpu.queue,
            gpu.config.format,
            gpu.config.width,
            gpu.config.height,
            samples,
        );
        let mut caption = Caption::new(&gpu.device, gpu.config.format);
        caption.set_recording_progress(&gpu.queue, None, Duration::ZERO);
        log::info!(
            "Desktop adapter: {} — {}x{}, {samples}x multisampling",
            gpu.adapter.get_info().name,
            gpu.config.width,
            gpu.config.height
        );
        Ok(Self {
            window,
            gpu,
            renderer,
            caption,
            size,
            occluded: false,
            participant,
            swarm: Swarm::new(),
            signals: Default::default(),
            pointer: Pointer::default(),
            hand_menu: HandMenu::default(),
            motion: arrival_motion::Motion::default(),
            arrival_time: 0.0,
            delegation_scene: crate::delegation::Scene::new(),
            workers: Default::default(),
            coordinator_samples: Vec::with_capacity(crate::swarm::AGENT_COUNT),
            presentation_navigation: Default::default(),
            panel: Default::default(),
            last_frame: Instant::now(),
            first_frame: true,
        })
    }

    fn resize(&mut self, size: PhysicalSize<u32>) {
        self.size = size;
        if size.width == 0 || size.height == 0 {
            return;
        }
        self.gpu.config.width = size.width;
        self.gpu.config.height = size.height;
        self.gpu
            .surface
            .configure(&self.gpu.device, &self.gpu.config);
        self.renderer
            .resize(&self.gpu.device, size.width, size.height);
        self.window.request_redraw();
    }

    /// One pass of the headset's frame loop (`xr::run`), from the keyboard and mouse instead of the controllers, drawing it if `render`.
    fn frame(&mut self, render: bool) -> Result<()> {
        let now = Instant::now();
        let dt = now.duration_since(self.last_frame).as_secs_f32().min(0.1);
        self.last_frame = now;
        let p = &mut self.participant;
        let focused = p.focused;
        p.connection.poll(focused, &mut p.conversation);
        let conversation = &mut p.conversation;
        let input = &mut p.input;
        let (a, back, y, mute, replay) = (
            std::mem::take(&mut input.a),
            std::mem::take(&mut input.back),
            std::mem::take(&mut input.y),
            std::mem::take(&mut input.mute),
            std::mem::take(&mut input.replay),
        );
        let stick = Input::stick(input.left);
        let moving = stick.x.abs() > STICK || stick.y.abs() > STICK;
        if moving && !p.stick_latched {
            if stick.y.abs() >= stick.x.abs() {
                conversation.navigate(0, if stick.y > 0.0 { -1 } else { 1 });
            } else {
                conversation.navigate(if stick.x > 0.0 { 1 } else { -1 }, 0);
            }
        }
        p.stick_latched = moving;
        if back {
            conversation.back();
        }
        // Only a hold whose press is in a space can leave it, as on the headset.
        if back && conversation.joined_space().is_some() {
            p.back_since = Some(now);
        }
        match p.back_since {
            Some(since) if input.back_held && conversation.joined_space().is_some() => {
                conversation.hold_back(Some(now.duration_since(since)));
            }
            Some(_) => {
                p.back_since = None;
                conversation.hold_back(None);
            }
            None => {}
        }
        conversation.tick_space(now);
        if y {
            conversation.refresh();
        }
        let choosing = conversation.is_choosing();
        let right = Input::stick(input.right);
        let choice_moving = choosing && right.x.abs() > STICK;
        if choice_moving && !p.choice_latched {
            conversation.choose(if right.x > 0.0 { 1 } else { -1 });
        }
        p.choice_latched = choice_moving;
        let selecting = conversation.selects_queue();
        let queue_moving = selecting && right.y.abs() > STICK && right.y.abs() > right.x.abs();
        if queue_moving && !p.queue_latched {
            conversation.select_queued(if right.y > 0.0 { -1 } else { 1 });
        }
        p.queue_latched = queue_moving;
        // A chooses what was pointed at last frame, and only in the hand menu while it shows; see `xr::run`.
        match &p.highlighted {
            Some(arrival::Key::Menu(key)) if a => {
                if let Some(choice) = self.hand_menu.choose(key) {
                    conversation.choose_menu(choice);
                }
            }
            _ if p.menu_was_shown => {}
            _ if a => {
                conversation.press_a(p.highlighted.as_ref());
            }
            _ => {}
        }
        if focused && !input.talk {
            p.talk_blocked = false;
        }
        let talking = focused && input.talk && !p.talk_blocked;
        if talking != p.was_talking {
            conversation.talk(talking);
        }
        p.was_talking = talking;
        if mute {
            conversation.toggle_mute();
        }
        if replay {
            conversation.replay();
        }
        self.presentation_navigation.update(
            None,
            &crate::delegation::Workspace::default(),
            if choosing { 0.0 } else { right.x },
            if choosing || selecting { 0.0 } else { right.y },
        );
        // Knocks are announced in the title as well, since there is no wrist to cue while the menu is closed.
        let knocks = conversation.knocks().len();
        if knocks != p.knocks {
            p.knocks = knocks;
            for knock in conversation.knocks() {
                log::info!("{} is knocking: + admits, - declines.", knock.name);
            }
            self.window.set_title(&match conversation.knocks().first() {
                Some(knock) => {
                    format!("{TITLE} — {} is knocking: + admits, - declines", knock.name)
                }
                None => TITLE.to_owned(),
            });
        }
        let queue = &self.gpu.queue;
        self.caption.set_text(queue, &conversation.text());
        self.caption
            .set_recording_progress(queue, conversation.recording(), Duration::ZERO);
        let visual = self.signals.update(
            crate::interaction::Inputs {
                session: if focused {
                    conversation.activity()
                } else {
                    crate::interaction::SessionActivity::Unknown
                },
                ..Default::default()
            },
            dt,
        );
        self.swarm.set_visual(visual, dt);

        let size = Vec2::new(
            self.size.width.max(1) as f32,
            self.size.height.max(1) as f32,
        );
        let head = p.head.transform();
        let ray = p
            .input
            .cursor
            .filter(|_| focused)
            .and_then(|cursor| p.head.ray(cursor, size));
        self.arrival_time += dt;
        self.motion
            .update(&crate::xr::arrival_target(conversation), self.arrival_time);
        let mut scene = self.motion.scene(self.arrival_time);
        let palm = p.menu_open.then(|| p.head.palm());
        let eye = arrival::HOME;
        let menu_shown = focused && self.hand_menu.update(palm, Some(eye), Instant::now());
        p.menu_was_shown = menu_shown;
        let menu = |hand_menu: &HandMenu, conversation: &Conversation| match palm {
            Some(palm) if menu_shown => hand_menu.objects(&palm, &conversation.menu_model(), eye),
            _ => Vec::new(),
        };
        scene.objects.extend(menu(&self.hand_menu, conversation));
        scene.viewer = Some(eye);
        let targets: Vec<_> = scene
            .targets()
            .into_iter()
            .filter(|target| !menu_shown || matches!(target.key, arrival::Key::Menu(_)))
            .collect();
        let trigger = focused.then_some(if p.input.trigger { 1.0 } else { 0.0 });
        let pointing = self.pointer.update(ray, trigger, &targets);
        p.highlighted = pointing.hit.as_ref().map(|hit| hit.key.clone());
        if let Some(key) = &pointing.chosen {
            match key {
                arrival::Key::Menu(key) => {
                    if let Some(choice) = self.hand_menu.choose(key) {
                        conversation.choose_menu(choice);
                    }
                }
                key => conversation.choose_object(key),
            }
            self.motion
                .update(&crate::xr::arrival_target(conversation), self.arrival_time);
            scene = self.motion.scene(self.arrival_time);
            scene.objects.extend(menu(&self.hand_menu, conversation));
            scene.viewer = Some(eye);
        }
        let in_arrival = conversation.arrival().is_some();
        self.renderer
            .set_scenery(self.motion.scenery(self.arrival_time));
        let attracting = pointing.attracting && !menu_shown;
        // The mouse attracts the swarm to a point along its ray, at about arm's length further than a controller would be.
        let attractor = ray.map(|ray| ray.at(1.2)).filter(|_| attracting);
        self.swarm.step(dt, attractor);
        self.renderer
            .update(queue, &self.swarm, attractor, attracting);
        let (root, current_workspace) = self.workers.update(conversation.joined_space());
        self.renderer
            .set_presentation_page(self.presentation_navigation.card);
        self.renderer
            .set_presentation_content_page(self.presentation_navigation.content);
        self.coordinator_samples.clear();
        self.coordinator_samples.extend(
            self.swarm
                .agents
                .iter()
                .map(|agent| agent.position.to_array()),
        );
        self.delegation_scene
            .set_coordinator_samples(&self.coordinator_samples);
        self.delegation_scene.update(root, current_workspace, dt);
        self.renderer.update_workspace(
            &self.gpu.device,
            queue,
            self.delegation_scene.cubes(),
            self.delegation_scene.stations(),
            current_workspace,
        );
        if in_arrival || menu_shown || !scene.objects.is_empty() || scene.heading.is_some() {
            self.renderer.update_arrival(
                &self.gpu.device,
                queue,
                &arrival_graphics::Frame {
                    scene: &scene,
                    hovered: pointing.hit.as_ref().map(|hit| &hit.key),
                    // The mouse cursor already shows where it points, so no ray is drawn from the eye.
                    ray: None,
                    time: self.arrival_time,
                },
            );
        }
        // The others in a space see this participant's head; a desktop has no controllers or hands to report. Visitors send no presence; see `design/security.md`.
        if conversation.role().participates()
            && let Some(pose) = controller::Pose::from_transform(head)
        {
            p.connection.send_presence(pose, None, None, [None, None]);
        }
        if in_arrival {
            self.panel.reset();
        }
        let panel_pose = self.panel.update(head, dt);
        if !render {
            return Ok(());
        }
        let Some(frame) = acquire(&self.gpu.surface, &self.gpu.device, &self.gpu.config)? else {
            return Ok(());
        };
        let target = frame.texture.create_view(&Default::default());
        self.renderer.render(
            &self.gpu.device,
            &self.gpu.queue,
            &[target],
            &[p.head.view_projection(size.x / size.y)],
            Mat4::IDENTITY,
            // As on the headset: arrival has no panel, and the hand menu hides it.
            (!in_arrival && !menu_shown).then_some((&self.caption, panel_pose)),
        );
        self.window.pre_present_notify();
        self.gpu.queue.present(frame);
        if self.first_frame {
            log::info!("Presented first desktop frame");
            self.first_frame = false;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_head_starts_at_home_looking_towards_arrival() {
        let head = Head::default();
        let transform = head.transform();
        assert!(transform.w_axis.truncate().distance(arrival::HOME) < 1e-6);
        let forward = transform.transform_vector3(-Vec3::Z);
        // Ahead, along -Z, where arrival's objects are, and a little downwards.
        assert!(forward.z < -0.9);
        assert!(forward.y < 0.0);
    }

    #[test]
    fn the_centre_of_the_window_points_where_the_head_looks() {
        let head = Head {
            yaw: 0.7,
            pitch: 0.2,
        };
        let size = Vec2::new(1280.0, 800.0);
        let ray = head.ray(size * 0.5, size).unwrap();
        let forward = head.transform().transform_vector3(-Vec3::Z);
        assert!(ray.direction.dot(forward) > 0.999);
        // From the eye, give or take the near plane.
        assert!(ray.origin.distance(arrival::HOME) < 0.1);
    }

    #[test]
    fn looking_turns_the_head_and_never_past_straight_up_or_down() {
        let mut head = Head::default();
        head.look(Vec2::new(100.0, 0.0));
        assert!(head.yaw < 0.0, "dragging right turns right");
        head.look(Vec2::new(0.0, -100_000.0));
        assert!(head.pitch <= 1.4);
        head.look(Vec2::new(0.0, 100_000.0));
        assert!(head.pitch >= -1.4);
    }

    #[test]
    fn the_hand_menu_palm_is_in_view_and_faces_the_eye() {
        let head = Head {
            yaw: -1.2,
            pitch: 0.3,
        };
        let palm = head.palm();
        let to_eye = (arrival::HOME - palm.wrist).normalize();
        assert!(palm.normal.dot(to_eye) > 0.999);
        // Within the 60° vertical field of view, so the menu shows in the window.
        let size = Vec2::new(1280.0, 800.0);
        let clip = head
            .view_projection(size.x / size.y)
            .project_point3(palm.wrist);
        assert!(clip.x.abs() < 1.0 && clip.y.abs() < 1.0 && (0.0..1.0).contains(&clip.z));
    }

    #[test]
    fn arrow_keys_and_brackets_are_sticks() {
        assert_eq!(Input::stick([true, false, false, false]), Vec2::Y);
        assert_eq!(Input::stick([false, false, false, true]), Vec2::X);
        assert_eq!(Input::stick([true, true, true, true]), Vec2::ZERO);
    }
}
